//! The SDL bridge (roadmap step E1, docs/SDL_BRIDGE.md).
//!
//! The game loads `libSDL3.so.0` from its own directory; cs2metal serves its shim
//! there instead. The shim's constructor asks the host, through the private
//! system call `0x43530002`, to fill its table by name with host thunks into an
//! x86-64 SDL 3 for macOS, loaded into this process, so that windows, input, and
//! audio are macOS's. Both are SDL 3, whose ABI is the same on x86-64 Linux and
//! macOS, so most functions pass through unchanged. A few are wrapped here:
//! those that take guest callbacks, which must run in the guest domain, and those
//! that take Linux paths.

use std::collections::HashMap;
use std::ffi::{CStr, CString, c_char, c_void};
use std::sync::Mutex;

use cs2metal_loader::Bridge;

/// The private system call the shim makes.
pub(crate) const BIND_SDL: u64 = 0x4353_0002;

/// Where the shim lives, set at start.
pub(crate) static SHIM: Mutex<Option<CString>> = Mutex::new(None);

/// The guest path the shim answers for: any `libSDL3.so.0`.
pub(crate) fn redirect(path: &CStr) -> Option<CString> {
    let bytes = path.to_bytes();
    (bytes == b"libSDL3.so.0" || bytes.ends_with(b"/libSDL3.so.0"))
        .then(|| SHIM.lock().ok().and_then(|s| s.clone()))
        .flatten()
}

struct State {
    bridge: Bridge,
    /// The host library's own functions, by name.
    real: HashMap<String, usize>,
    /// Event watches the guest added, by its (filter, userdata).
    watches: HashMap<(u64, u64), Box<Callback>>,
}

// SAFETY: the raw handles are used only under the STATE lock.
unsafe impl Send for State {}

static STATE: Mutex<Option<State>> = Mutex::new(None);

/// A guest callback and its userdata, as host SDL calls it through a wrapper.
struct Callback {
    /// The callback's guest stub, callable from host code.
    stub: u64,
    userdata: u64,
}

/// The newest x86-64 SDL 3 for macOS available: `CS2METAL_SDL3`, else the one the
/// newest Vulkan SDK ships (a universal framework).
fn host_library() -> Option<String> {
    if let Ok(path) = std::env::var("CS2METAL_SDL3") {
        return Some(path);
    }
    let sdk = vulkan_sdk()?;
    Some(format!(
        "{sdk}/lib/SDL3.xcframework/macos-arm64_x86_64/SDL3.framework/SDL3"
    ))
}

/// The newest Vulkan SDK's `macOS` directory: `VULKAN_SDK`, else under
/// `~/VulkanSDK`.
pub(crate) fn vulkan_sdk() -> Option<String> {
    if let Ok(sdk) = std::env::var("VULKAN_SDK") {
        return Some(sdk);
    }
    let home = std::env::var("HOME").ok()?;
    let mut versions: Vec<String> = std::fs::read_dir(format!("{home}/VulkanSDK"))
        .ok()?
        .filter_map(|e| e.ok()?.file_name().into_string().ok())
        .filter(|n| n.starts_with(|c: char| c.is_ascii_digit()))
        .collect();
    versions.sort_by_key(|v| {
        v.split('.')
            .map(|p| p.parse::<u32>().unwrap_or(0))
            .collect::<Vec<_>>()
    });
    let newest = versions.pop()?;
    Some(format!("{home}/VulkanSDK/{newest}/macOS"))
}

/// Whether the calling host thread runs a guest thread, so that a guest callback
/// can run on it.
fn has_guest() -> bool {
    crate::syscall::BLOCK.with(|b| b.try_borrow().is_ok_and(|b| b.is_some()))
}

fn real(name: &str) -> usize {
    STATE
        .lock()
        .ok()
        .and_then(|s| s.as_ref().and_then(|s| s.real.get(name).copied()))
        .unwrap_or(0)
}

fn guest_stub(callback: u64) -> u64 {
    if callback == 0 {
        return 0;
    }
    STATE
        .lock()
        .ok()
        .and_then(|mut s| s.as_mut().and_then(|s| s.bridge.guest(callback).ok()))
        .unwrap_or(0)
}

// --- Log output --------------------------------------------------------------------

static LOG: Mutex<Option<(u64, u64)>> = Mutex::new(None);

type LogOutput = extern "C" fn(u64, i32, i32, *const c_char);

/// Host SDL's log output function while the guest has one: the guest's, on a
/// guest thread; standard error on any other thread.
extern "C" fn log_output(_userdata: u64, category: i32, priority: i32, message: *const c_char) {
    let guest = LOG.lock().ok().and_then(|l| *l);
    if let Some((stub, userdata)) = guest.filter(|_| has_guest()) {
        // SAFETY: the stub enters the guest's SDL_LogOutputFunction.
        let f: LogOutput = unsafe { std::mem::transmute(stub as usize) };
        f(userdata, category, priority, message);
    } else if !message.is_null() {
        // SAFETY: SDL passes a C string.
        let text = unsafe { CStr::from_ptr(message) }.to_string_lossy();
        eprintln!("SDL: {text}");
    }
}

/// `SDL_SetLogOutputFunction(callback, userdata)`.
extern "C" fn set_log_output_function(callback: u64, userdata: u64) {
    let f = real("SDL_SetLogOutputFunction");
    if f == 0 {
        return;
    }
    // SAFETY: the host library's SDL_SetLogOutputFunction.
    let f: extern "C" fn(u64, u64) = unsafe { std::mem::transmute(f) };
    if callback == 0 {
        if let Ok(mut log) = LOG.lock() {
            *log = None;
        }
        f(0, 0);
        return;
    }
    let stub = guest_stub(callback);
    if let Ok(mut log) = LOG.lock() {
        *log = Some((stub, userdata));
    }
    f(log_output as *const () as u64, 0);
}

// --- Event watches -----------------------------------------------------------------

type EventFilter = extern "C" fn(u64, *mut c_void) -> bool;

/// Host SDL's event watch for one of the guest's: the guest's, on a guest
/// thread. An event pushed on another thread does not reach it.
extern "C" fn event_watch(record: *const Callback, event: *mut c_void) -> bool {
    if !has_guest() {
        return true;
    }
    // SAFETY: the record lives while the watch is registered.
    let record = unsafe { &*record };
    // SAFETY: the stub enters the guest's SDL_EventFilter.
    let f: EventFilter = unsafe { std::mem::transmute(record.stub as usize) };
    f(record.userdata, event)
}

/// `SDL_AddEventWatch(filter, userdata)`.
extern "C" fn add_event_watch(filter: u64, userdata: u64) -> bool {
    let f = real("SDL_AddEventWatch");
    let stub = guest_stub(filter);
    if f == 0 || stub == 0 {
        return false;
    }
    let record = Box::new(Callback { stub, userdata });
    let pointer = std::ptr::from_ref(record.as_ref()) as u64;
    if let Ok(mut s) = STATE.lock() {
        if let Some(s) = s.as_mut() {
            s.watches.insert((filter, userdata), record);
        }
    }
    // SAFETY: the host library's SDL_AddEventWatch.
    let f: extern "C" fn(u64, u64) -> bool = unsafe { std::mem::transmute(f) };
    f(event_watch as *const () as u64, pointer)
}

/// `SDL_RemoveEventWatch(filter, userdata)`.
extern "C" fn remove_event_watch(filter: u64, userdata: u64) {
    let f = real("SDL_RemoveEventWatch");
    let record = STATE.lock().ok().and_then(|mut s| {
        s.as_mut()
            .and_then(|s| s.watches.remove(&(filter, userdata)))
    });
    if let (Some(record), true) = (record, f != 0) {
        // SAFETY: the host library's SDL_RemoveEventWatch.
        let f: extern "C" fn(u64, u64) = unsafe { std::mem::transmute(f) };
        f(
            event_watch as *const () as u64,
            std::ptr::from_ref(record.as_ref()) as u64,
        );
    }
}

// --- Clipboard ---------------------------------------------------------------------

/// `SDL_SetClipboardData(callback, cleanup, userdata, mime_types, count)`: host
/// SDL calls both callbacks on the thread that owns the clipboard, the guest's
/// main thread.
extern "C" fn set_clipboard_data(
    callback: u64,
    cleanup: u64,
    userdata: u64,
    mime_types: u64,
    count: usize,
) -> bool {
    let f = real("SDL_SetClipboardData");
    if f == 0 {
        return false;
    }
    // SAFETY: the host library's SDL_SetClipboardData.
    let f: extern "C" fn(u64, u64, u64, u64, usize) -> bool = unsafe { std::mem::transmute(f) };
    f(
        guest_stub(callback),
        guest_stub(cleanup),
        userdata,
        mime_types,
        count,
    )
}

// --- Audio -------------------------------------------------------------------------

/// `SDL_OpenAudioDeviceStream(device, spec, callback, userdata)`. Host SDL calls
/// a stream's callback on its own audio thread, which runs no guest thread; the
/// game feeds its streams with `SDL_PutAudioStreamData` instead, so a callback is
/// not bridged yet, and named if one comes.
extern "C" fn open_audio_device_stream(
    device: u32,
    spec: u64,
    callback: u64,
    userdata: u64,
) -> u64 {
    let f = real("SDL_OpenAudioDeviceStream");
    if f == 0 {
        return 0;
    }
    if callback != 0 {
        eprintln!("cs2metal: SDL audio stream callback not bridged; the stream opens without it");
    }
    // SAFETY: the host library's SDL_OpenAudioDeviceStream.
    let f: extern "C" fn(u32, u64, u64, u64) -> u64 = unsafe { std::mem::transmute(f) };
    f(device, spec, 0, if callback != 0 { 0 } else { userdata })
}

// --- Paths -------------------------------------------------------------------------

/// `SDL_IOFromFile(file, mode)`: the guest's path, through the installation
/// overlay.
extern "C" fn io_from_file(file: *const c_char, mode: *const c_char) -> u64 {
    let f = real("SDL_IOFromFile");
    if f == 0 || file.is_null() || mode.is_null() {
        return 0;
    }
    // SAFETY: the guest passes C strings.
    let (path, how) = unsafe { (CStr::from_ptr(file), CStr::from_ptr(mode).to_bytes()) };
    let how = match how.first() {
        Some(b'w') => crate::paths::Use::Replace,
        Some(b'a') => crate::paths::Use::Modify,
        _ if how.contains(&b'+') => crate::paths::Use::Modify,
        _ => crate::paths::Use::Read,
    };
    let Ok(path) = crate::paths::resolve(path, libc::AT_FDCWD, how) else {
        return 0;
    };
    // SAFETY: the host library's SDL_IOFromFile.
    let f: extern "C" fn(*const c_char, *const c_char) -> u64 = unsafe { std::mem::transmute(f) };
    f(path.as_ptr(), mode)
}

/// The macOS Vulkan loader host SDL should load: `CS2METAL_VULKAN_LOADER`, else
/// the newest Vulkan SDK's.
pub(crate) fn vulkan_loader() -> Option<String> {
    std::env::var("CS2METAL_VULKAN_LOADER")
        .ok()
        .or_else(|| Some(format!("{}/lib/libvulkan.1.dylib", vulkan_sdk()?)))
}

/// Points the macOS Vulkan loader at the SDK's MoltenVK manifest, in the host
/// environment, unless the owner names drivers.
pub(crate) fn prepare_vulkan_environment() {
    if std::env::var_os("VK_DRIVER_FILES").is_some()
        || std::env::var_os("VK_ICD_FILENAMES").is_some()
    {
        return;
    }
    if let Some(sdk) = vulkan_sdk() {
        // SAFETY: the host environment; the guest's is separate.
        unsafe {
            std::env::set_var(
                "VK_DRIVER_FILES",
                format!("{sdk}/share/vulkan/icd.d/MoltenVK_icd.json"),
            );
        }
    }
}

/// `SDL_Vulkan_LoadLibrary(path)`: the guest names its Linux loader, or none;
/// host SDL loads the macOS loader.
extern "C" fn vulkan_load_library(_path: *const c_char) -> bool {
    let f = real("SDL_Vulkan_LoadLibrary");
    if f == 0 {
        return false;
    }
    let loader = vulkan_loader().and_then(|l| CString::new(l).ok());
    // SAFETY: the host library's SDL_Vulkan_LoadLibrary.
    let f: extern "C" fn(*const c_char) -> bool = unsafe { std::mem::transmute(f) };
    f(loader.as_ref().map_or(std::ptr::null(), |l| l.as_ptr()))
}

// --- Binding -----------------------------------------------------------------------

/// A function the host library lacks, named when called.
extern "C" fn missing() -> u64 {
    eprintln!("cs2metal: an SDL function the host's SDL lacks was called");
    0
}

/// The host wrapper for `name`, if it has one.
fn wrapper(name: &[u8]) -> Option<u64> {
    let f: *const () = match name {
        b"SDL_SetLogOutputFunction" => set_log_output_function as *const (),
        b"SDL_AddEventWatch" => add_event_watch as *const (),
        b"SDL_RemoveEventWatch" => remove_event_watch as *const (),
        b"SDL_SetClipboardData" => set_clipboard_data as *const (),
        b"SDL_OpenAudioDeviceStream" => open_audio_device_stream as *const (),
        b"SDL_IOFromFile" => io_from_file as *const (),
        b"SDL_Vulkan_LoadLibrary" => vulkan_load_library as *const (),
        _ => return None,
    };
    Some(f as u64)
}

/// The private system call: fills the shim's `table` of `count` entries for the
/// NUL-terminated names at `names`.
pub(crate) fn bind(table: u64, names: u64, count: u64) -> i64 {
    let Some(path) = host_library().and_then(|p| CString::new(p).ok()) else {
        eprintln!("cs2metal: no SDL 3 for macOS found; set CS2METAL_SDL3");
        return crate::translate::ENOENT;
    };
    // SAFETY: dlopen of an SDL 3 library, in the host domain.
    let library = unsafe { libc::dlopen(path.as_ptr(), libc::RTLD_NOW | libc::RTLD_LOCAL) };
    if library.is_null() {
        eprintln!("cs2metal: the host's SDL 3 did not load from {path:?}");
        return crate::translate::ENOENT;
    }
    prepare_vulkan_environment();
    let mut bridge = match Bridge::new() {
        Ok(b) => b,
        Err(_) => return crate::translate::ENOMEM,
    };
    let mut real = HashMap::new();
    let mut bound = 0;
    let mut lacking = Vec::new();
    for i in 0..count {
        // SAFETY: the shim passes `count` string pointers and a table as long.
        let name = unsafe { CStr::from_ptr(*((names + 8 * i) as *const *const c_char)) };
        // SAFETY: dlsym on the handle just opened.
        let symbol = unsafe { libc::dlsym(library, name.as_ptr()) } as u64;
        if symbol != 0 {
            real.insert(name.to_string_lossy().into_owned(), symbol as usize);
            bound += 1;
        } else {
            lacking.push(name.to_string_lossy().into_owned());
        }
        let target = match wrapper(name.to_bytes()) {
            Some(w) if symbol != 0 => w,
            _ if symbol != 0 => symbol,
            _ => missing as *const () as u64,
        };
        let Ok(stub) = bridge.host(target) else {
            return crate::translate::ENOMEM;
        };
        // SAFETY: the shim's table has `count` entries.
        unsafe { std::ptr::write((table + 8 * i) as *mut u64, stub) };
    }
    if let Ok(mut state) = STATE.lock() {
        *state = Some(State {
            bridge,
            real,
            watches: HashMap::new(),
        });
    }
    eprintln!(
        "cs2metal: SDL bridge bound {bound} of {count} functions to {}",
        path.to_string_lossy()
    );
    if !lacking.is_empty() && std::env::var_os("CS2METAL_SDL_TRACE").is_some() {
        eprintln!("cs2metal: the host's SDL lacks {}", lacking.join(", "));
    }
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_shim_answers_for_libsdl3_only() {
        if let Ok(mut shim) = SHIM.lock() {
            *shim = Some(CString::new("/shims/libSDL3.so.0").expect("path"));
        }
        let at = |p: &str| redirect(&CString::new(p).expect("path"));
        assert!(at("/game/bin/linuxsteamrt64/libSDL3.so.0").is_some());
        assert!(at("libSDL3.so.0").is_some());
        assert!(at("/game/bin/linuxsteamrt64/libSDL3.so.0.1").is_none());
        assert!(at("/x/libSDL2-2.0.so.0").is_none());
    }

    #[test]
    fn every_wrapped_function_has_a_wrapper() {
        for name in [
            "SDL_SetLogOutputFunction",
            "SDL_AddEventWatch",
            "SDL_RemoveEventWatch",
            "SDL_SetClipboardData",
            "SDL_OpenAudioDeviceStream",
            "SDL_IOFromFile",
            "SDL_Vulkan_LoadLibrary",
        ] {
            assert!(wrapper(name.as_bytes()).is_some(), "{name}");
        }
        assert!(wrapper(b"SDL_Init").is_none());
    }
}
