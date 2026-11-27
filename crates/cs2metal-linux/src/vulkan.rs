//! The Vulkan bridge (roadmap step F2, docs/VULKAN_BRIDGE.md).
//!
//! The renderer loads `libvulkan.so.1` and reaches every function through
//! `vkGetInstanceProcAddr`. cs2metal serves its shim at that name; the shim's
//! constructor asks the host, through the private system call `0x43530003`, to
//! fill its table with entries into the macOS Vulkan loader, with MoltenVK beneath
//! it. Every function pointer the loader hands out reaches the guest as a host
//! thunk. Handles and structures cross unchanged: both sides are x86-64 System V.
//!
//! Three calls are adjusted on the way. `vkCreateInstance` enables portability
//! enumeration, without which the loader hides MoltenVK, and drops extensions the
//! host does not have. `vkCreateDevice` enables `VK_KHR_portability_subset`,
//! which the specification requires on a device that offers it. A debug
//! messenger's callback is guest code and is called through the guest thunk.

use std::collections::HashSet;
use std::ffi::{CStr, CString, c_char};
use std::sync::Mutex;

use cs2metal_loader::Bridge;

/// The private system call the shim makes.
pub(crate) const BIND_VULKAN: u64 = 0x4353_0003;

/// Where the shim lives, set at start.
pub(crate) static SHIM: Mutex<Option<CString>> = Mutex::new(None);

/// The guest paths the shim answers for: the loader's names.
pub(crate) fn redirect(path: &CStr) -> Option<CString> {
    let bytes = path.to_bytes();
    let name = bytes.rsplit(|&b| b == b'/').next().unwrap_or(bytes);
    matches!(name, b"libvulkan.so.1" | b"libvulkan.so")
        .then(|| SHIM.lock().ok().and_then(|s| s.clone()))
        .flatten()
}

struct State {
    bridge: Bridge,
    get_instance_proc_addr: usize,
    get_device_proc_addr: usize,
}

// SAFETY: the raw handles are used only under the STATE lock.
unsafe impl Send for State {}

static STATE: Mutex<Option<State>> = Mutex::new(None);

type GetProcAddr = extern "C" fn(u64, *const c_char) -> u64;

const VK_SUCCESS: i32 = 0;
const VK_ERROR_INITIALIZATION_FAILED: i32 = -3;
const PORTABILITY_ENUMERATION: &CStr = c"VK_KHR_portability_enumeration";
const PORTABILITY_SUBSET: &CStr = c"VK_KHR_portability_subset";
const ENUMERATE_PORTABILITY_BIT: u32 = 0x1;
const DEBUG_UTILS_MESSENGER_CREATE_INFO: u32 = 1_000_128_004;
const DEBUG_REPORT_CALLBACK_CREATE_INFO: u32 = 1_000_011_000;

fn tracing() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("CS2METAL_VULKAN_TRACE").is_some())
}

fn host_entry(f: u64) -> u64 {
    if f == 0 {
        return 0;
    }
    STATE
        .lock()
        .ok()
        .and_then(|mut s| s.as_mut().and_then(|s| s.bridge.host(f).ok()))
        .unwrap_or(0)
}

fn guest_entry(f: u64) -> u64 {
    if f == 0 {
        return 0;
    }
    STATE
        .lock()
        .ok()
        .and_then(|mut s| s.as_mut().and_then(|s| s.bridge.guest(f).ok()))
        .unwrap_or(0)
}

fn real(which: fn(&State) -> usize) -> usize {
    STATE
        .lock()
        .ok()
        .and_then(|s| s.as_ref().map(which))
        .unwrap_or(0)
}

/// A host function of the loader, by name, for `instance` (0 for the global
/// ones).
fn loader_function(instance: u64, name: &CStr) -> u64 {
    let gipa = real(|s| s.get_instance_proc_addr);
    if gipa == 0 {
        return 0;
    }
    // SAFETY: the loader's vkGetInstanceProcAddr.
    let gipa: GetProcAddr = unsafe { std::mem::transmute(gipa) };
    gipa(instance, name.as_ptr())
}

/// The host wrapper for a function the bridge adjusts, by name.
fn wrapper(name: &[u8]) -> Option<u64> {
    let f: *const () = match name {
        b"vkGetInstanceProcAddr" | b"vk_icdGetInstanceProcAddr" => {
            get_instance_proc_addr as *const ()
        }
        b"vkGetDeviceProcAddr" => get_device_proc_addr as *const (),
        b"vkCreateInstance" => create_instance as *const (),
        b"vkCreateDevice" => create_device as *const (),
        b"vkCreateDebugUtilsMessengerEXT" => create_debug_utils_messenger as *const (),
        b"vkCreateDebugReportCallbackEXT" => create_debug_report_callback as *const (),
        b"vkEnumerateDeviceExtensionProperties" => enumerate_device_extensions as *const (),
        _ => return None,
    };
    Some(f as u64)
}

/// What the guest gets for a function the loader made: the wrapper's entry, or
/// the loader's own function behind a host thunk.
fn guest_function(name: &CStr, host: u64) -> u64 {
    let always = matches!(
        name.to_bytes(),
        b"vkGetInstanceProcAddr" | b"vk_icdGetInstanceProcAddr"
    );
    if host == 0 && !always {
        return 0;
    }
    match wrapper(name.to_bytes()) {
        Some(w) => host_entry(w),
        None => host_entry(host),
    }
}

/// `vkGetInstanceProcAddr(instance, name)`.
extern "C" fn get_instance_proc_addr(instance: u64, name: *const c_char) -> u64 {
    if name.is_null() {
        return 0;
    }
    // SAFETY: the guest passes a C string.
    let name = unsafe { CStr::from_ptr(name) };
    guest_function(name, loader_function(instance, name))
}

/// `vkGetDeviceProcAddr(device, name)`.
extern "C" fn get_device_proc_addr(device: u64, name: *const c_char) -> u64 {
    let gdpa = real(|s| s.get_device_proc_addr);
    if name.is_null() || gdpa == 0 {
        return 0;
    }
    // SAFETY: the loader's vkGetDeviceProcAddr, and a C string from the guest.
    let (gdpa, name): (GetProcAddr, &CStr) = unsafe {
        (
            std::mem::transmute::<usize, GetProcAddr>(gdpa),
            CStr::from_ptr(name),
        )
    };
    guest_function(name, gdpa(device, name.as_ptr()))
}

/// The C strings of a `count`-long array of pointers.
///
/// # Safety
///
/// `list` must be null or hold `count` pointers to C strings.
unsafe fn strings<'a>(list: u64, count: u32) -> Vec<&'a CStr> {
    if list == 0 {
        return Vec::new();
    }
    (0..u64::from(count))
        // SAFETY: the caller's contract.
        .map(|i| unsafe { CStr::from_ptr(*((list + 8 * i) as *const *const c_char)) })
        .collect()
}

/// The extension names an enumeration function reports.
fn extension_names(enumerate: impl Fn(*mut u32, *mut u8) -> i32) -> HashSet<CString> {
    const PROPERTIES: usize = 260; // VkExtensionProperties: a 256-byte name, a version
    let mut count = 0u32;
    if enumerate(&mut count, std::ptr::null_mut()) != VK_SUCCESS {
        return HashSet::new();
    }
    let mut buf = vec![0u8; PROPERTIES * count as usize];
    if enumerate(&mut count, buf.as_mut_ptr()) < 0 {
        return HashSet::new();
    }
    buf.chunks(PROPERTIES)
        .take(count as usize)
        .filter_map(|p| {
            CStr::from_bytes_until_nul(&p[..256])
                .ok()
                .map(CStr::to_owned)
        })
        .collect()
}

fn join(names: &[&CStr]) -> String {
    names
        .iter()
        .map(|n| n.to_string_lossy())
        .collect::<Vec<_>>()
        .join(" ")
}

/// Swaps the guest callbacks in the debug structures of a `pNext` chain for
/// their guest entries, returning what to restore after the call.
///
/// # Safety
///
/// `next` must be null or a valid Vulkan structure chain in guest memory.
unsafe fn wrap_debug_callbacks(mut next: u64) -> Vec<(u64, u64)> {
    let mut saved = Vec::new();
    while next != 0 {
        // SAFETY: every Vulkan structure starts with sType and pNext.
        let (kind, following) = unsafe {
            (
                std::ptr::read_unaligned(next as *const u32),
                std::ptr::read_unaligned((next + 8) as *const u64),
            )
        };
        let field = match kind {
            DEBUG_UTILS_MESSENGER_CREATE_INFO => Some(next + 32),
            DEBUG_REPORT_CALLBACK_CREATE_INFO => Some(next + 24),
            _ => None,
        };
        if let Some(field) = field {
            // SAFETY: the callback field of the structure just identified.
            unsafe {
                let callback = std::ptr::read_unaligned(field as *const u64);
                saved.push((field, callback));
                std::ptr::write_unaligned(field as *mut u64, guest_entry(callback));
            }
        }
        next = following;
    }
    saved
}

/// # Safety
///
/// `saved` must come from [`wrap_debug_callbacks`] on structures still alive.
unsafe fn restore(saved: Vec<(u64, u64)>) {
    for (field, value) in saved {
        // SAFETY: the caller's contract.
        unsafe { std::ptr::write_unaligned(field as *mut u64, value) };
    }
}

/// `vkCreateInstance(info, allocator, instance)`.
extern "C" fn create_instance(info: u64, _allocator: u64, out: u64) -> i32 {
    let create = loader_function(0, c"vkCreateInstance");
    let enumerate = loader_function(0, c"vkEnumerateInstanceExtensionProperties");
    if info == 0 || create == 0 || enumerate == 0 {
        return VK_ERROR_INITIALIZATION_FAILED;
    }
    // SAFETY: the loader's vkEnumerateInstanceExtensionProperties.
    let enumerate: extern "C" fn(*const c_char, *mut u32, *mut u8) -> i32 =
        unsafe { std::mem::transmute(enumerate as usize) };
    let available = extension_names(|n, p| enumerate(std::ptr::null(), n, p));
    // VkInstanceCreateInfo: flags at 16, extension count at 48, names at 56.
    let mut copy = [0u8; 64];
    // SAFETY: the guest passes a VkInstanceCreateInfo of 64 bytes.
    unsafe { std::ptr::copy_nonoverlapping(info as *const u8, copy.as_mut_ptr(), 64) };
    let word = |o: usize| u64::from_le_bytes(copy[o..o + 8].try_into().unwrap_or([0; 8]));
    let (next, names) = (word(8), word(56));
    let count = u32::from_le_bytes(copy[48..52].try_into().unwrap_or([0; 4]));
    // SAFETY: the create info's extension names.
    let requested = unsafe { strings(names, count) };
    let (mut kept, dropped): (Vec<&CStr>, Vec<&CStr>) =
        requested.iter().partition(|n| available.contains(**n));
    let portability = available.contains(PORTABILITY_ENUMERATION);
    if portability && !kept.contains(&PORTABILITY_ENUMERATION) {
        kept.push(PORTABILITY_ENUMERATION);
    }
    if tracing() || !dropped.is_empty() {
        eprintln!(
            "cs2metal: vkCreateInstance: extensions {}{}",
            join(&requested),
            if dropped.is_empty() {
                String::new()
            } else {
                format!("; dropped, absent on the host: {}", join(&dropped))
            }
        );
    }
    let pointers: Vec<*const c_char> = kept.iter().map(|n| n.as_ptr()).collect();
    let mut flags = u32::from_le_bytes(copy[16..20].try_into().unwrap_or([0; 4]));
    if portability {
        flags |= ENUMERATE_PORTABILITY_BIT;
    }
    copy[16..20].copy_from_slice(&flags.to_le_bytes());
    copy[48..52].copy_from_slice(&(pointers.len() as u32).to_le_bytes());
    copy[56..64].copy_from_slice(&(pointers.as_ptr() as u64).to_le_bytes());
    // SAFETY: the create info's pNext chain.
    let saved = unsafe { wrap_debug_callbacks(next) };
    // SAFETY: the loader's vkCreateInstance, with the adjusted copy. Allocation
    // callbacks are guest code and are not passed: the loader uses its own.
    let create: extern "C" fn(*const u8, u64, u64) -> i32 =
        unsafe { std::mem::transmute(create as usize) };
    let result = create(copy.as_ptr(), 0, out);
    // SAFETY: the structures wrapped above.
    unsafe { restore(saved) };
    if result == VK_SUCCESS {
        remember_instance(out);
    }
    if tracing() || result != VK_SUCCESS {
        eprintln!("cs2metal: vkCreateInstance returned {result}");
    }
    result
}

/// `vkCreateDevice(physical_device, info, allocator, device)`.
extern "C" fn create_device(physical: u64, info: u64, _allocator: u64, out: u64) -> i32 {
    // vkCreateDevice and vkEnumerateDeviceExtensionProperties are instance-level;
    // the loader dispatches them by the physical device's instance, so a null
    // instance does not do. They are looked up through the trampolines instead.
    let create = loader_function(INSTANCE.lock().map_or(0, |i| *i), c"vkCreateDevice");
    let enumerate = loader_function(
        INSTANCE.lock().map_or(0, |i| *i),
        c"vkEnumerateDeviceExtensionProperties",
    );
    if info == 0 || create == 0 || enumerate == 0 {
        return VK_ERROR_INITIALIZATION_FAILED;
    }
    // SAFETY: the loader's vkEnumerateDeviceExtensionProperties.
    let enumerate: extern "C" fn(u64, *const c_char, *mut u32, *mut u8) -> i32 =
        unsafe { std::mem::transmute(enumerate as usize) };
    let available = extension_names(|n, p| enumerate(physical, std::ptr::null(), n, p));
    // VkDeviceCreateInfo: extension count at 48, names at 56; 72 bytes.
    let mut copy = [0u8; 72];
    // SAFETY: the guest passes a VkDeviceCreateInfo of 72 bytes.
    unsafe { std::ptr::copy_nonoverlapping(info as *const u8, copy.as_mut_ptr(), 72) };
    let count = u32::from_le_bytes(copy[48..52].try_into().unwrap_or([0; 4]));
    let names = u64::from_le_bytes(copy[56..64].try_into().unwrap_or([0; 8]));
    // SAFETY: the create info's extension names.
    let requested = unsafe { strings(names, count) };
    let mut kept = requested.clone();
    if available.contains(PORTABILITY_SUBSET) && !kept.contains(&PORTABILITY_SUBSET) {
        kept.push(PORTABILITY_SUBSET);
    }
    let missing: Vec<&CStr> = requested
        .iter()
        .copied()
        .filter(|n| !available.contains(*n))
        .collect();
    if tracing() || !missing.is_empty() {
        eprintln!(
            "cs2metal: vkCreateDevice: extensions {}{}",
            join(&requested),
            if missing.is_empty() {
                String::new()
            } else {
                format!("; absent on the device: {}", join(&missing))
            }
        );
    }
    let pointers: Vec<*const c_char> = kept.iter().map(|n| n.as_ptr()).collect();
    copy[48..52].copy_from_slice(&(pointers.len() as u32).to_le_bytes());
    copy[56..64].copy_from_slice(&(pointers.as_ptr() as u64).to_le_bytes());
    // SAFETY: the loader's vkCreateDevice, with the adjusted copy.
    let create: extern "C" fn(u64, *const u8, u64, u64) -> i32 =
        unsafe { std::mem::transmute(create as usize) };
    let result = create(physical, copy.as_ptr(), 0, out);
    if tracing() || result != VK_SUCCESS {
        eprintln!("cs2metal: vkCreateDevice returned {result}");
    }
    result
}

/// Device extensions hidden from the game: `CS2METAL_VULKAN_HIDE`, a comma-separated
/// list.
fn hidden_extensions() -> &'static [CString] {
    static HIDDEN: std::sync::OnceLock<Vec<CString>> = std::sync::OnceLock::new();
    HIDDEN.get_or_init(|| {
        std::env::var("CS2METAL_VULKAN_HIDE")
            .unwrap_or_default()
            .split(',')
            .filter(|n| !n.is_empty())
            .filter_map(|n| CString::new(n.trim()).ok())
            .collect()
    })
}

/// `vkEnumerateDeviceExtensionProperties(physical, layer, count, properties)`,
/// without the hidden extensions.
extern "C" fn enumerate_device_extensions(
    physical: u64,
    layer: *const c_char,
    count: *mut u32,
    properties: *mut u8,
) -> i32 {
    const PROPERTIES: usize = 260;
    let f = loader_function(
        INSTANCE.lock().map_or(0, |i| *i),
        c"vkEnumerateDeviceExtensionProperties",
    );
    if f == 0 || count.is_null() {
        return VK_ERROR_INITIALIZATION_FAILED;
    }
    // SAFETY: the loader's vkEnumerateDeviceExtensionProperties.
    let f: extern "C" fn(u64, *const c_char, *mut u32, *mut u8) -> i32 =
        unsafe { std::mem::transmute(f as usize) };
    let hidden = hidden_extensions();
    if hidden.is_empty() {
        return f(physical, layer, count, properties);
    }
    let mut all = 0u32;
    if f(physical, layer, &mut all, std::ptr::null_mut()) != VK_SUCCESS {
        return VK_ERROR_INITIALIZATION_FAILED;
    }
    let mut buf = vec![0u8; PROPERTIES * all as usize];
    let result = f(physical, layer, &mut all, buf.as_mut_ptr());
    if result < 0 {
        return result;
    }
    let kept: Vec<&[u8]> = buf
        .chunks(PROPERTIES)
        .take(all as usize)
        .filter(|p| {
            CStr::from_bytes_until_nul(&p[..256])
                .map_or(true, |n| !hidden.iter().any(|h| h.as_c_str() == n))
        })
        .collect();
    // SAFETY: the guest passes a count, and room for that many when properties
    // is not null.
    unsafe {
        if properties.is_null() {
            *count = kept.len() as u32;
            return VK_SUCCESS;
        }
        let room = (*count as usize).min(kept.len());
        for (i, p) in kept.iter().take(room).enumerate() {
            std::ptr::copy_nonoverlapping(p.as_ptr(), properties.add(i * PROPERTIES), PROPERTIES);
        }
        *count = room as u32;
        if room < kept.len() { 5 } else { VK_SUCCESS } // VK_INCOMPLETE
    }
}

/// The instance the game created, for instance-level lookups the bridge makes.
static INSTANCE: Mutex<u64> = Mutex::new(0);

/// `vkCreateDebugUtilsMessengerEXT(instance, info, allocator, messenger)`.
extern "C" fn create_debug_utils_messenger(instance: u64, info: u64, _a: u64, out: u64) -> i32 {
    debug_create(instance, info, out, c"vkCreateDebugUtilsMessengerEXT")
}

/// `vkCreateDebugReportCallbackEXT(instance, info, allocator, callback)`.
extern "C" fn create_debug_report_callback(instance: u64, info: u64, _a: u64, out: u64) -> i32 {
    debug_create(instance, info, out, c"vkCreateDebugReportCallbackEXT")
}

fn debug_create(instance: u64, info: u64, out: u64, name: &CStr) -> i32 {
    let create = loader_function(instance, name);
    if create == 0 || info == 0 {
        return VK_ERROR_INITIALIZATION_FAILED;
    }
    // The create info itself heads the chain the wrapper walks.
    // SAFETY: a debug create info from the guest.
    let saved = unsafe { wrap_debug_callbacks(info) };
    // SAFETY: the loader's creation function of that name.
    let create: extern "C" fn(u64, u64, u64, u64) -> i32 =
        unsafe { std::mem::transmute(create as usize) };
    let result = create(instance, info, 0, out);
    // SAFETY: the structure wrapped above.
    unsafe { restore(saved) };
    result
}

/// Records instances as the guest creates them, through the instance wrapper's
/// output.
fn remember_instance(out: u64) {
    if out != 0 {
        // SAFETY: vkCreateInstance wrote the handle.
        let instance = unsafe { std::ptr::read_unaligned(out as *const u64) };
        if let Ok(mut i) = INSTANCE.lock() {
            *i = instance;
        }
    }
}

/// The private system call: fills the shim's `table` of `count` entries for the
/// NUL-terminated names at `names`.
pub(crate) fn bind(table: u64, names: u64, count: u64) -> i64 {
    crate::sdl::prepare_vulkan_environment();
    let Some(path) = crate::sdl::vulkan_loader().and_then(|p| CString::new(p).ok()) else {
        eprintln!("cs2metal: no Vulkan loader for macOS found; set CS2METAL_VULKAN_LOADER");
        return crate::translate::ENOENT;
    };
    // SAFETY: dlopen of the Vulkan loader, in the host domain.
    let library = unsafe { libc::dlopen(path.as_ptr(), libc::RTLD_NOW | libc::RTLD_LOCAL) };
    if library.is_null() {
        eprintln!("cs2metal: the macOS Vulkan loader did not load from {path:?}");
        return crate::translate::ENOENT;
    }
    // SAFETY: dlsym on the handle just opened.
    let (gipa, gdpa) = unsafe {
        (
            libc::dlsym(library, c"vkGetInstanceProcAddr".as_ptr()) as usize,
            libc::dlsym(library, c"vkGetDeviceProcAddr".as_ptr()) as usize,
        )
    };
    if gipa == 0 {
        return crate::translate::ENOENT;
    }
    let Ok(bridge) = Bridge::new() else {
        return crate::translate::ENOMEM;
    };
    if let Ok(mut state) = STATE.lock() {
        *state = Some(State {
            bridge,
            get_instance_proc_addr: gipa,
            get_device_proc_addr: gdpa,
        });
    }
    for i in 0..count {
        // SAFETY: the shim passes `count` string pointers and a table as long.
        let name = unsafe { CStr::from_ptr(*((names + 8 * i) as *const *const c_char)) };
        let entry = guest_function(name, loader_function(0, name));
        // SAFETY: the shim's table has `count` entries.
        unsafe { std::ptr::write((table + 8 * i) as *mut u64, entry) };
    }
    eprintln!(
        "cs2metal: Vulkan bridge bound to {}",
        path.to_string_lossy()
    );
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_shim_answers_for_the_loader_names() {
        if let Ok(mut shim) = SHIM.lock() {
            *shim = Some(CString::new("/shims/libvulkan.so.1").expect("path"));
        }
        let at = |p: &str| redirect(&CString::new(p).expect("path"));
        assert!(at("libvulkan.so.1").is_some());
        assert!(at("/runtime/lib/x86_64-linux-gnu/libvulkan.so.1").is_some());
        assert!(at("/runtime/lib/x86_64-linux-gnu/libvulkan.so.1.4.309").is_none());
        assert!(at("/game/bin/librendersystemvulkan.so").is_none());
    }

    /// Creates an instance and a device through the wrappers, on the owner's
    /// macOS loader and MoltenVK, as the renderer would. Skipped without a Vulkan
    /// SDK.
    #[cfg(target_arch = "x86_64")]
    #[test]
    fn an_instance_and_a_device_come_up_on_moltenvk() {
        let Some(loader) = crate::sdl::vulkan_loader().filter(|l| std::path::Path::new(l).exists())
        else {
            eprintln!("no Vulkan SDK; skipped");
            return;
        };
        let names: Vec<CString> = ["vkGetInstanceProcAddr", "vkCreateInstance"]
            .iter()
            .map(|n| CString::new(*n).expect("name"))
            .collect();
        let pointers: Vec<*const c_char> = names.iter().map(|n| n.as_ptr()).collect();
        let mut table = [0u64; 2];
        assert_eq!(
            bind(table.as_mut_ptr() as u64, pointers.as_ptr() as u64, 2),
            0,
            "{loader}"
        );
        assert!(table.iter().all(|&e| e != 0), "both entries bound");

        // VkInstanceCreateInfo with no extensions: sType 1, the rest zero.
        let mut info = [0u8; 64];
        info[0] = 1;
        let mut instance = 0u64;
        assert_eq!(
            create_instance(
                info.as_ptr() as u64,
                0,
                std::ptr::from_mut(&mut instance) as u64
            ),
            VK_SUCCESS,
            "portability enumeration lets the loader offer MoltenVK"
        );
        let enumerate = loader_function(instance, c"vkEnumeratePhysicalDevices");
        // SAFETY: the loader's vkEnumeratePhysicalDevices.
        let enumerate: extern "C" fn(u64, *mut u32, *mut u64) -> i32 =
            unsafe { std::mem::transmute(enumerate as usize) };
        let mut count = 1u32;
        let mut physical = 0u64;
        assert!(enumerate(instance, &mut count, &mut physical) >= 0);
        assert_eq!(count, 1, "a device");

        // One queue of family 0, priority 1.0.
        let priority = 1.0f32;
        let mut queue = [0u8; 40];
        queue[0..4].copy_from_slice(&2u32.to_le_bytes()); // VK_STRUCTURE_TYPE_DEVICE_QUEUE_CREATE_INFO
        queue[24..28].copy_from_slice(&1u32.to_le_bytes());
        queue[32..40].copy_from_slice(&(std::ptr::from_ref(&priority) as u64).to_le_bytes());
        let mut device_info = [0u8; 72];
        device_info[0..4].copy_from_slice(&3u32.to_le_bytes()); // VK_STRUCTURE_TYPE_DEVICE_CREATE_INFO
        device_info[20..24].copy_from_slice(&1u32.to_le_bytes());
        device_info[24..32].copy_from_slice(&(queue.as_ptr() as u64).to_le_bytes());
        let mut device = 0u64;
        assert_eq!(
            create_device(
                physical,
                device_info.as_ptr() as u64,
                0,
                std::ptr::from_mut(&mut device) as u64
            ),
            VK_SUCCESS,
            "with VK_KHR_portability_subset added"
        );
        assert_ne!(device, 0);
    }

    #[test]
    fn adjusted_functions_have_wrappers() {
        for name in [
            "vkGetInstanceProcAddr",
            "vkGetDeviceProcAddr",
            "vkCreateInstance",
            "vkCreateDevice",
            "vkCreateDebugUtilsMessengerEXT",
        ] {
            assert!(wrapper(name.as_bytes()).is_some(), "{name}");
        }
        assert!(wrapper(b"vkQueueSubmit").is_none());
    }
}
