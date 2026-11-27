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
        b"vkCmdBindDescriptorSets" => bind_descriptor_sets as *const (),
        b"vkCreateSwapchainKHR" if tracing() => create_swapchain as *const (),
        b"vkAcquireNextImageKHR" if tracing() => acquire_next_image as *const (),
        b"vkQueuePresentKHR" if tracing() => queue_present as *const (),
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
    if name.to_bytes() == b"vkCmdBindDescriptorSets" {
        BIND_DESCRIPTOR_SETS.store(host, std::sync::atomic::Ordering::Release);
    }
    if tracing() && host != 0 {
        if let Some(w) = traced_wrapper(name.to_bytes(), host) {
            return host_entry(w);
        }
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

/// The loader's `vkCmdBindDescriptorSets`, recorded when the guest asks for it.
static BIND_DESCRIPTOR_SETS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

type BindDescriptorSets = extern "C" fn(u64, u32, u64, u32, u32, *const u64, u32, *const u32);

/// The runs of consecutive sets that are not null, as (first index, length).
fn bound_runs(sets: &[u64]) -> Vec<(usize, usize)> {
    let mut runs: Vec<(usize, usize)> = Vec::new();
    for (i, &set) in sets.iter().enumerate() {
        if set == 0 {
            continue;
        }
        match runs.last_mut() {
            Some((start, len)) if *start + *len == i => *len += 1,
            _ => runs.push((i, 1)),
        }
    }
    runs
}

/// `vkCmdBindDescriptorSets(commandBuffer, bindPoint, layout, firstSet, count,
/// sets, dynamicOffsetCount, dynamicOffsets)`: null sets are left out.
///
/// The renderer binds ranges of sets with `VK_NULL_HANDLE` among them. The
/// Linux drivers skip those; MoltenVK reads through the null pointer when it
/// encodes the command, and the thread that submits the frame never returns.
/// So each run of sets that are not null is bound by a call of its own, which
/// leaves the numbers in between as they were, as a skipped set does.
extern "C" fn bind_descriptor_sets(
    command_buffer: u64,
    bind_point: u32,
    layout: u64,
    first: u32,
    count: u32,
    sets: *const u64,
    dynamic_count: u32,
    dynamic: *const u32,
) {
    let real = BIND_DESCRIPTOR_SETS.load(std::sync::atomic::Ordering::Acquire);
    if real == 0 {
        return;
    }
    // SAFETY: the loader's vkCmdBindDescriptorSets.
    let real = unsafe { std::mem::transmute::<usize, BindDescriptorSets>(real as usize) };
    let handles: &[u64] = if sets.is_null() || count == 0 {
        &[]
    } else {
        // SAFETY: the guest passes `count` set handles.
        unsafe { std::slice::from_raw_parts(sets, count as usize) }
    };
    if !handles.contains(&0) {
        return real(
            command_buffer,
            bind_point,
            layout,
            first,
            count,
            sets,
            dynamic_count,
            dynamic,
        );
    }
    let runs = bound_runs(handles);
    static NAMED: std::sync::Once = std::sync::Once::new();
    NAMED.call_once(|| {
        eprintln!(
            "cs2metal: vkCmdBindDescriptorSets with null sets ({} of {count} bound, from set \
             {first}, {dynamic_count} dynamic offsets); the null ones are left out",
            runs.iter().map(|r| r.1).sum::<usize>()
        );
    });
    // Dynamic offsets go to the sets that have dynamic descriptors, in order,
    // and a null set has none. Which run takes how many is in the set layouts,
    // which the bridge does not keep: one run takes them all, several are
    // named once and the first takes them.
    if runs.len() > 1 && dynamic_count > 0 {
        static SPLIT: std::sync::Once = std::sync::Once::new();
        SPLIT.call_once(|| {
            eprintln!(
                "cs2metal: vkCmdBindDescriptorSets: {dynamic_count} dynamic offsets across \
                 {} runs of sets; given to the first",
                runs.len()
            );
        });
    }
    for (i, &(start, len)) in runs.iter().enumerate() {
        let (offsets, offset_list) = if i == 0 {
            (dynamic_count, dynamic)
        } else {
            (0, std::ptr::null())
        };
        real(
            command_buffer,
            bind_point,
            layout,
            first + start as u32,
            len as u32,
            // SAFETY: inside the guest's array of `count` handles.
            unsafe { sets.add(start) },
            offsets,
            offset_list,
        );
    }
}

/// The loader's device function `name`, for the traced wrappers.
fn device_function(device: u64, name: &CStr) -> u64 {
    let gdpa = real(|s| s.get_device_proc_addr);
    if gdpa == 0 {
        return 0;
    }
    // SAFETY: the loader's vkGetDeviceProcAddr.
    let gdpa: GetProcAddr = unsafe { std::mem::transmute(gdpa) };
    gdpa(device, name.as_ptr())
}

static DEVICE: Mutex<u64> = Mutex::new(0);

/// Counts calls and results of a traced function, logging the first few and
/// every change of result.
fn count(what: &str, result: i32) {
    static SEEN: Mutex<Vec<(String, i32, u64)>> = Mutex::new(Vec::new());
    let Ok(mut seen) = SEEN.lock() else {
        return;
    };
    let entry = seen.iter_mut().find(|(w, _, _)| w == what);
    match entry {
        Some((_, last, n)) => {
            *n += 1;
            if *last != result || n.is_power_of_two() {
                eprintln!("cs2metal: {what} returned {result} (call {n})");
                *last = result;
            }
        }
        None => {
            eprintln!("cs2metal: {what} returned {result} (call 1)");
            seen.push((what.to_owned(), result, 1));
        }
    }
}

/// `vkCreateSwapchainKHR`, traced: the extent, format, and present mode.
extern "C" fn create_swapchain(device: u64, info: u64, allocator: u64, out: u64) -> i32 {
    if let Ok(mut d) = DEVICE.lock() {
        *d = device;
    }
    let f = device_function(device, c"vkCreateSwapchainKHR");
    if f == 0 || info == 0 {
        return VK_ERROR_INITIALIZATION_FAILED;
    }
    // VkSwapchainCreateInfoKHR: minImageCount at 32, format 36, color space 40,
    // extent 44 and 48, present mode at 88 (after the queue family fields).
    // SAFETY: the guest passes a VkSwapchainCreateInfoKHR.
    let field = |o: u64| unsafe { std::ptr::read_unaligned((info + o) as *const u32) };
    eprintln!(
        "cs2metal: vkCreateSwapchainKHR images {} format {} colorspace {} extent {}x{} present mode {}",
        field(32),
        field(36),
        field(40),
        field(44),
        field(48),
        field(88)
    );
    // SAFETY: the loader's vkCreateSwapchainKHR.
    let f: extern "C" fn(u64, u64, u64, u64) -> i32 = unsafe { std::mem::transmute(f as usize) };
    let result = f(device, info, allocator, out);
    count("vkCreateSwapchainKHR", result);
    result
}

/// `vkAcquireNextImageKHR`, traced.
extern "C" fn acquire_next_image(
    device: u64,
    swapchain: u64,
    timeout: u64,
    semaphore: u64,
    fence: u64,
    index: u64,
) -> i32 {
    let f = device_function(device, c"vkAcquireNextImageKHR");
    if f == 0 {
        return VK_ERROR_INITIALIZATION_FAILED;
    }
    // SAFETY: the loader's vkAcquireNextImageKHR.
    let f: extern "C" fn(u64, u64, u64, u64, u64, u64) -> i32 =
        unsafe { std::mem::transmute(f as usize) };
    let result = f(device, swapchain, timeout, semaphore, fence, index);
    count("vkAcquireNextImageKHR", result);
    result
}

/// How often the trace reports the pacing of presents.
const PACING_PERIOD: std::time::Duration = std::time::Duration::from_secs(10);

/// The intervals between presents since the last report, in microseconds.
struct Pacing {
    last: Option<std::time::Instant>,
    since: Option<std::time::Instant>,
    intervals: Vec<u32>,
}

static PACING: Mutex<Pacing> = Mutex::new(Pacing {
    last: None,
    since: None,
    intervals: Vec::new(),
});

/// The report's line for a period's intervals between presents: how many, the
/// median, the 99th percentile, the longest, and how many took more than twice
/// the median, which is what a stutter is.
fn pacing_line(intervals: &mut [u32]) -> Option<String> {
    if intervals.is_empty() {
        return None;
    }
    intervals.sort_unstable();
    let at = |per_mille: usize| intervals[(intervals.len() - 1) * per_mille / 1000];
    let median = at(500);
    let slow = intervals.iter().filter(|&&i| i > 2 * median).count();
    let ms = |micros: u32| f64::from(micros) / 1000.0;
    Some(format!(
        "cs2metal: presents: {} in the period, median {:.1} ms, 99th percentile {:.1} ms, \
         longest {:.1} ms, {slow} over twice the median",
        intervals.len(),
        ms(median),
        ms(at(990)),
        ms(at(1000)),
    ))
}

/// Records a present for the pacing reports.
fn note_present() {
    let now = std::time::Instant::now();
    let Ok(mut pacing) = PACING.lock() else {
        return;
    };
    if let Some(last) = pacing.last {
        let micros = now.duration_since(last).as_micros();
        pacing
            .intervals
            .push(u32::try_from(micros).unwrap_or(u32::MAX));
    }
    pacing.last = Some(now);
    let since = *pacing.since.get_or_insert(now);
    if now.duration_since(since) >= PACING_PERIOD {
        pacing.since = Some(now);
        let mut intervals = std::mem::take(&mut pacing.intervals);
        if let Some(line) = pacing_line(&mut intervals) {
            eprintln!("{line}");
        }
    }
}

/// `vkQueuePresentKHR`, traced.
extern "C" fn queue_present(queue: u64, info: u64) -> i32 {
    note_present();
    let device = DEVICE.lock().map_or(0, |d| *d);
    let f = device_function(device, c"vkQueuePresentKHR");
    if f == 0 {
        return VK_ERROR_INITIALIZATION_FAILED;
    }
    // SAFETY: the loader's vkQueuePresentKHR.
    let f: extern "C" fn(u64, u64) -> i32 = unsafe { std::mem::transmute(f as usize) };
    let result = f(queue, info);
    count("vkQueuePresentKHR", result);
    result
}

/// The loader's functions behind the generic traced wrappers, by name.
static TRACED: Mutex<Vec<(&'static str, u64)>> = Mutex::new(Vec::new());

fn traced_real(name: &str) -> u64 {
    TRACED
        .lock()
        .ok()
        .and_then(|t| t.iter().find(|(n, _)| *n == name).map(|(_, f)| *f))
        .unwrap_or(0)
}

/// A wrapper that calls the loader's function `$name` with up to six integer
/// arguments and counts its results.
macro_rules! traced {
    ($wrapper:ident, $name:literal) => {
        extern "C" fn $wrapper(a: u64, b: u64, c: u64, d: u64, e: u64, f: u64) -> i32 {
            let real = traced_real($name);
            if real == 0 {
                return VK_ERROR_INITIALIZATION_FAILED;
            }
            type Six = extern "C" fn(u64, u64, u64, u64, u64, u64) -> i32;
            // SAFETY: the loader's function of that name, which takes at most six
            // integer arguments; extra ones are ignored by the ABI.
            let real = unsafe { std::mem::transmute::<usize, Six>(real as usize) };
            let result = real(a, b, c, d, e, f);
            count($name, result);
            result
        }
    };
}

traced!(traced_queue_submit, "vkQueueSubmit");
traced!(traced_queue_submit2, "vkQueueSubmit2");
traced!(traced_queue_submit2_khr, "vkQueueSubmit2KHR");
traced!(traced_wait_for_fences, "vkWaitForFences");
traced!(traced_get_fence_status, "vkGetFenceStatus");
traced!(traced_wait_semaphores, "vkWaitSemaphores");
traced!(traced_wait_semaphores_khr, "vkWaitSemaphoresKHR");
traced!(traced_semaphore_value, "vkGetSemaphoreCounterValue");
traced!(traced_semaphore_value_khr, "vkGetSemaphoreCounterValueKHR");
traced!(traced_device_wait_idle, "vkDeviceWaitIdle");
traced!(traced_queue_wait_idle, "vkQueueWaitIdle");
traced!(traced_wait_for_present, "vkWaitForPresentKHR");

/// The generic traced wrapper for `name`, recording the loader's function.
fn traced_wrapper(name: &[u8], host: u64) -> Option<u64> {
    let (key, f): (&'static str, *const ()) = match name {
        b"vkQueueSubmit" => ("vkQueueSubmit", traced_queue_submit as *const ()),
        b"vkQueueSubmit2" => ("vkQueueSubmit2", traced_queue_submit2 as *const ()),
        b"vkQueueSubmit2KHR" => ("vkQueueSubmit2KHR", traced_queue_submit2_khr as *const ()),
        b"vkWaitForFences" => ("vkWaitForFences", traced_wait_for_fences as *const ()),
        b"vkGetFenceStatus" => ("vkGetFenceStatus", traced_get_fence_status as *const ()),
        b"vkWaitSemaphores" => ("vkWaitSemaphores", traced_wait_semaphores as *const ()),
        b"vkWaitSemaphoresKHR" => (
            "vkWaitSemaphoresKHR",
            traced_wait_semaphores_khr as *const (),
        ),
        b"vkGetSemaphoreCounterValue" => (
            "vkGetSemaphoreCounterValue",
            traced_semaphore_value as *const (),
        ),
        b"vkGetSemaphoreCounterValueKHR" => (
            "vkGetSemaphoreCounterValueKHR",
            traced_semaphore_value_khr as *const (),
        ),
        b"vkDeviceWaitIdle" => ("vkDeviceWaitIdle", traced_device_wait_idle as *const ()),
        b"vkQueueWaitIdle" => ("vkQueueWaitIdle", traced_queue_wait_idle as *const ()),
        b"vkWaitForPresentKHR" => ("vkWaitForPresentKHR", traced_wait_for_present as *const ()),
        _ => return None,
    };
    if let Ok(mut traced) = TRACED.lock() {
        if !traced.iter().any(|(n, _)| *n == key) {
            traced.push((key, host));
        }
    }
    Some(f as u64)
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
    fn a_pacing_report_names_the_stutters() {
        assert!(pacing_line(&mut []).is_none());
        // 98 presents at 10 ms, one at 25 ms, one at 300 ms.
        let mut intervals = vec![10_000u32; 98];
        intervals.extend([300_000, 25_000]);
        assert_eq!(
            pacing_line(&mut intervals).expect("line"),
            "cs2metal: presents: 100 in the period, median 10.0 ms, 99th percentile 25.0 ms, \
             longest 300.0 ms, 2 over twice the median"
        );
    }

    #[test]
    fn null_descriptor_sets_split_a_binding_into_runs() {
        assert_eq!(bound_runs(&[1, 2, 3]), [(0, 3)]);
        assert_eq!(bound_runs(&[0, 2, 3, 0, 5, 0]), [(1, 2), (4, 1)]);
        assert_eq!(bound_runs(&[0, 0]), []);
        assert_eq!(bound_runs(&[]), []);
    }

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
