//! The Steam bridge (roadmap step D2, docs/STEAM_BRIDGE_DESIGN.md).
//!
//! The game's own `libsteam_api.so` loads `~/.steam/sdk64/steamclient.so`, as on
//! Linux. cs2metal serves its shim library at that path, without writing anything
//! to the owner's home. The shim's constructor asks the host, through the private
//! system call `0x43530001`, to fill its table by name. Each entry is a fixing
//! thunk into the owner's macOS `steamclient.dylib`, loaded into this process; the
//! three names that library does not export are answered here without inventing
//! anything. Whatever a client function or method returns that is one of the
//! client library's objects reaches the guest as a proxy, whose methods cross the
//! bridge in turn.

use std::collections::HashMap;
use std::ffi::{CStr, CString, c_char, c_void};
use std::sync::Mutex;

use cs2metal_loader::{Bridge, set_return_fixup};

/// The private system call the shim makes.
pub(crate) const BIND_STEAM: u64 = 0x4353_0001;

/// Where the shim lives, set at start.
pub(crate) static SHIM: Mutex<Option<CString>> = Mutex::new(None);

struct State {
    bridge: Bridge,
    /// The client library's image base, for recognizing its objects by their
    /// vtables.
    base: usize,
    create_interface: usize,
    proxies: HashMap<u64, u64>,
}

// SAFETY: the raw handles are used only under the STATE lock.
unsafe impl Send for State {}

static STATE: Mutex<Option<State>> = Mutex::new(None);

/// The guest path the shim answers for: `~/.steam/sdk64/steamclient.so`.
pub(crate) fn redirect(path: &CStr) -> Option<CString> {
    path.to_bytes()
        .ends_with(b"/.steam/sdk64/steamclient.so")
        .then(|| SHIM.lock().ok().and_then(|s| s.clone()))
        .flatten()
}

fn client_library() -> String {
    std::env::var("CS2METAL_STEAMCLIENT").unwrap_or_else(|_| {
        let home = std::env::var("HOME").unwrap_or_default();
        format!(
            "{home}/Library/Application Support/Steam/Steam.AppBundle/Steam/Contents/MacOS/steamclient.dylib"
        )
    })
}

unsafe extern "C" {
    static mach_task_self_: u32;
    fn mach_vm_read_overwrite(task: u32, address: u64, size: u64, data: u64, out: *mut u64) -> i32;
    fn getsectiondata(
        header: *const c_void,
        segment: *const c_char,
        section: *const c_char,
        size: *mut libc::c_ulong,
    ) -> *mut u8;
}

/// Reads a word from this process's memory without faulting on a bad address.
fn safe_read(address: u64) -> Option<u64> {
    let mut value = 0u64;
    let mut size = 0u64;
    // SAFETY: the kernel copies 8 bytes into `value` or fails; nothing faults.
    let status = unsafe {
        mach_vm_read_overwrite(
            mach_task_self_,
            address,
            8,
            std::ptr::from_mut(&mut value) as u64,
            &mut size,
        )
    };
    (status == 0 && size == 8).then_some(value)
}

/// Whether `address` lies in the image whose base is `base`.
fn in_image(address: u64, base: usize) -> bool {
    // SAFETY: dladdr only reads the dynamic loader's own tables.
    let mut info: libc::Dl_info = unsafe { std::mem::zeroed() };
    // SAFETY: as above.
    let found = unsafe { libc::dladdr(address as *const c_void, &mut info) } != 0;
    found && info.dli_fbase as usize == base
}

fn tracing() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("CS2METAL_STEAM_TRACE").is_some())
}

static APP_ID_SET: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Gives the macOS client library, in this process's environment, the app id the
/// game declared in `steam_appid.txt`, once the game has written it. The game
/// writes the file after it first loads the client library (for crash reporting),
/// so this is retried after each client call until it succeeds.
fn mirror_app_id() {
    use std::sync::atomic::Ordering;
    if APP_ID_SET.load(Ordering::Relaxed) {
        return;
    }
    if let Some(id) = guest_app_id() {
        APP_ID_SET.store(true, Ordering::Relaxed);
        // SAFETY: the variables are read by the client library on this process's
        // threads; the guest's environment is separate and unchanged.
        unsafe {
            std::env::set_var("SteamAppId", id.to_string());
            std::env::set_var("SteamGameId", id.to_string());
        }
        eprintln!("cs2metal: Steam bridge: the game declares app {id}");
    }
}

/// The return fixup: a client library object becomes its guest proxy.
extern "C" fn fixup(value: u64) -> u64 {
    mirror_app_id();
    let fixed = fix(value);
    if tracing() {
        let what = describe_call();
        if fixed == value {
            eprintln!("cs2metal: steam {what} returned {value:#x}");
        } else {
            eprintln!("cs2metal: steam {what} returned object {value:#x} as proxy {fixed:#x}");
        }
    }
    fixed
}

/// Which client function or method the current thunk call reached: an exported
/// name, or the slot of a proxied object's vtable.
fn describe_call() -> String {
    let call = crate::syscall::BLOCK.with(|b| b.borrow().as_ref().and_then(|b| b.current_call()));
    let Some((target, _)) = call else {
        return "call".to_owned();
    };
    // SAFETY: Dl_info is plain data; dladdr fills it.
    let mut info: libc::Dl_info = unsafe { std::mem::zeroed() };
    // SAFETY: dladdr only reads the dynamic loader's tables.
    if unsafe { libc::dladdr(target as *const c_void, &mut info) } != 0
        && !info.dli_sname.is_null()
        && info.dli_saddr as u64 == target
    {
        // SAFETY: dladdr gave a symbol name.
        return unsafe { CStr::from_ptr(info.dli_sname) }
            .to_string_lossy()
            .into_owned();
    }
    let Ok(guard) = STATE.lock() else {
        return format!("{target:#x}");
    };
    let Some(state) = guard.as_ref() else {
        return format!("{target:#x}");
    };
    for &object in state.proxies.keys() {
        let Some(vtable) = safe_read(object) else {
            continue;
        };
        for slot in 0..cs2metal_loader::PROXY_SLOTS {
            if safe_read(vtable + 8 * slot) == Some(target) {
                return format!("object {object:#x} slot {slot}");
            }
        }
    }
    format!("{target:#x}")
}

fn fix(value: u64) -> u64 {
    // Handles, booleans, and Steam IDs are not user-space pointers.
    if value & 7 != 0 || !(0x1_0000_0000..0x7fff_ffff_ffff).contains(&value) {
        return value;
    }
    let Ok(mut guard) = STATE.lock() else {
        return value;
    };
    let Some(state) = guard.as_mut() else {
        return value;
    };
    if let Some(&proxy) = state.proxies.get(&value) {
        return proxy;
    }
    let Some(vtable) = safe_read(value) else {
        return value;
    };
    if !in_image(vtable, state.base) {
        return value;
    }
    let Ok(proxy_vtable) = state.bridge.proxy_vtable() else {
        return value;
    };
    let proxy = Box::leak(Box::new([proxy_vtable, value]));
    let proxy = std::ptr::from_ref(proxy) as u64;
    state.proxies.insert(value, proxy);
    proxy
}

/// `Steam_IsKnownInterface(version)`: whether the client library implements an
/// interface version. The macOS library does not export this function. Its
/// factory makes only the top-level interfaces, while the others (`SteamUtils`,
/// `SteamGameServer`, and so on) come from `ISteamClient` methods, so the
/// answer is whether the library names the version among its C strings, as it
/// does for every version it can hand out.
extern "C" fn is_known_interface(version: *const c_char) -> bool {
    if version.is_null() {
        return false;
    }
    // SAFETY: the guest passes a version string.
    let wanted = unsafe { CStr::from_ptr(version) }.to_bytes_with_nul();
    let base = STATE.lock().ok().and_then(|s| s.as_ref().map(|s| s.base));
    let Some(base) = base.filter(|&b| b != 0) else {
        return false;
    };
    let mut size: libc::c_ulong = 0;
    // SAFETY: `base` is the client library's Mach-O header, from dladdr.
    let data = unsafe {
        getsectiondata(
            base as *const c_void,
            c"__TEXT".as_ptr(),
            c"__cstring".as_ptr(),
            &mut size,
        )
    };
    if data.is_null() {
        return false;
    }
    // SAFETY: getsectiondata gave the mapped section and its size.
    let strings = unsafe { std::slice::from_raw_parts(data, size as usize) };
    names_string(strings, wanted)
}

/// Whether a section of NUL-terminated strings holds `wanted` (with its NUL) as
/// a whole string.
fn names_string(strings: &[u8], wanted: &[u8]) -> bool {
    strings
        .windows(wanted.len())
        .enumerate()
        .any(|(i, w)| w == wanted && (i == 0 || strings[i - 1] == 0))
}

/// `CreateInterface(version, status)` under `CS2METAL_STEAM_TRACE`: the client's
/// factory, with the version asked for logged.
extern "C" fn create_interface_traced(version: *const c_char, status: *mut i32) -> *mut c_void {
    let create = STATE
        .lock()
        .ok()
        .and_then(|s| s.as_ref().map(|s| s.create_interface))
        .unwrap_or(0);
    if create == 0 {
        return std::ptr::null_mut();
    }
    // SAFETY: CreateInterface(const char *, int *) is the client's factory.
    let create: extern "C" fn(*const c_char, *mut i32) -> *mut c_void =
        unsafe { std::mem::transmute(create) };
    let made = create(version, status);
    if !version.is_null() {
        // SAFETY: the guest passes a version string.
        let v = unsafe { CStr::from_ptr(version) };
        eprintln!("cs2metal: steam CreateInterface({v:?}) made {made:p}");
    }
    made
}

/// `Steam_NotifyMissingInterface(pipe, version)`: logged.
extern "C" fn notify_missing_interface(_pipe: i32, version: *const c_char) {
    if !version.is_null() {
        // SAFETY: the guest passes a version string.
        let v = unsafe { CStr::from_ptr(version) };
        eprintln!("cs2metal: the game asked for a missing Steam interface {v:?}");
    }
}

/// `Breakpad_*`: crash reports are not collected or sent from cs2metal.
extern "C" fn send_mini_dump() -> u64 {
    0
}

/// A client function the macOS library does not have, named when called.
extern "C" fn missing() -> u64 {
    eprintln!("cs2metal: a Steam client function the macOS library lacks was called");
    0
}

/// The app id the game wrote to `steam_appid.txt` beside its executable, as the
/// guest sees that file (through the installation overlay).
fn guest_app_id() -> Option<u32> {
    let exe = crate::syscall::EXE.lock().ok()?.clone();
    let dir = std::path::Path::new(&exe).parent()?;
    let file = CString::new(dir.join("steam_appid.txt").to_string_lossy().as_bytes()).ok()?;
    let path = crate::paths::resolve(&file, libc::AT_FDCWD, crate::paths::Use::Read).ok()?;
    let bytes = std::fs::read(path.to_str().ok()?).ok()?;
    // The game writes the digits, a newline, and a NUL byte.
    let digits: Vec<u8> = bytes
        .iter()
        .copied()
        .take_while(u8::is_ascii_digit)
        .collect();
    std::str::from_utf8(&digits).ok()?.parse().ok()
}

/// The private system call: fills the shim's `table` of `count` entries for the
/// NUL-terminated names at `names`.
pub(crate) fn bind(table: u64, names: u64, count: u64) -> i64 {
    // The macOS client library runs in this process and looks for the app id
    // where a macOS process keeps it, in its own environment, not in the
    // guest's.
    mirror_app_id();
    let path = CString::new(client_library()).unwrap_or_default();
    // SAFETY: dlopen of the owner's Steam client library, in the host domain.
    let library = unsafe { libc::dlopen(path.as_ptr(), libc::RTLD_NOW | libc::RTLD_LOCAL) };
    if library.is_null() {
        // SAFETY: dlerror returns a message or null.
        let why = unsafe { libc::dlerror() };
        let why = if why.is_null() {
            String::new()
        } else {
            // SAFETY: dlerror's message is a C string.
            unsafe { CStr::from_ptr(why) }
                .to_string_lossy()
                .into_owned()
        };
        eprintln!("cs2metal: the Steam client library did not load: {why}");
        return crate::translate::ENOENT;
    }
    let mut bridge = match Bridge::new() {
        Ok(b) => b,
        Err(_) => return crate::translate::ENOMEM,
    };
    // SAFETY: dlsym on the handle just opened.
    let create = unsafe { libc::dlsym(library, c"CreateInterface".as_ptr()) } as usize;
    // SAFETY: Dl_info is plain data; dladdr fills it.
    let mut info: libc::Dl_info = unsafe { std::mem::zeroed() };
    // SAFETY: dladdr on a symbol of the library.
    unsafe { libc::dladdr(create as *const c_void, &mut info) };
    let base = info.dli_fbase as usize;
    let mut bound = 0;
    for i in 0..count {
        // SAFETY: the shim passes `count` string pointers and a table as long.
        let name = unsafe { CStr::from_ptr(*((names + 8 * i) as *const *const c_char)) };
        // SAFETY: as above.
        let symbol = unsafe { libc::dlsym(library, name.as_ptr()) } as u64;
        let target = match name.to_bytes() {
            // Crash reporting stays out of this process: the macOS client's
            // Breakpad would install its own exception handlers over the bridge's
            // guest. Every Breakpad_* entry is a no-op returning zero.
            n if n.starts_with(b"Breakpad_") => send_mini_dump as *const () as u64,
            b"Steam_IsKnownInterface" => is_known_interface as *const () as u64,
            b"Steam_NotifyMissingInterface" => notify_missing_interface as *const () as u64,
            b"CreateInterface" if tracing() => create_interface_traced as *const () as u64,
            b"Breakpad_SteamSendMiniDump" => send_mini_dump as *const () as u64,
            _ if symbol != 0 => symbol,
            _ => missing as *const () as u64,
        };
        let Ok(stub) = bridge.host_fixed(target) else {
            return crate::translate::ENOMEM;
        };
        // SAFETY: the shim's table has `count` entries.
        unsafe { std::ptr::write((table + 8 * i) as *mut u64, stub) };
        bound += u64::from(symbol != 0);
    }
    if let Ok(mut state) = STATE.lock() {
        *state = Some(State {
            bridge,
            base,
            create_interface: create,
            proxies: HashMap::new(),
        });
    }
    set_return_fixup(fixup);
    eprintln!("cs2metal: Steam bridge bound {bound} of {count} client functions");
    0
}

#[cfg(test)]
mod tests {
    use super::names_string;

    #[test]
    fn a_version_must_be_a_whole_string() {
        let strings = b"SteamUtils010\0OtherSteamUser023\0\0";
        assert!(names_string(strings, b"SteamUtils010\0"));
        assert!(names_string(strings, b"OtherSteamUser023\0"));
        assert!(
            !names_string(strings, b"SteamUser023\0"),
            "a suffix is not a name"
        );
        assert!(!names_string(strings, b"SteamUtils011\0"));
    }
}
