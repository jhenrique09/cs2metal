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
    if tracing() && !event.is_null() {
        // SAFETY: SDL passes the event it is adding.
        note_event("an event watch", unsafe { event_kind(event as u64) });
    }
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

// --- Events and input, traced --------------------------------------------------------

/// Counts an event of type `kind` that reached the game by way of `source`, and
/// names the first of each kind and then every power of two, for the trace.
fn note_event(source: &'static str, kind: u32) {
    static SEEN: Mutex<Vec<(&'static str, u32, u64)>> = Mutex::new(Vec::new());
    let Ok(mut seen) = SEEN.lock() else {
        return;
    };
    let n = match seen.iter_mut().find(|(s, k, _)| *s == source && *k == kind) {
        Some((_, _, n)) => {
            *n += 1;
            *n
        }
        None => {
            seen.push((source, kind, 1));
            1
        }
    };
    if n.is_power_of_two() {
        eprintln!("cs2metal: SDL event {kind:#x} by {source} ({n} times)");
    }
}

/// The window the game made, for the trace's state lines.
static WINDOW: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// A host function of no arguments or of the game's window, called for the
/// trace; 0 if the host library lacks it.
fn window_state(name: &str, window: Option<u64>) -> u64 {
    let f = real(name);
    if f == 0 {
        return 0;
    }
    match window {
        Some(w) => {
            // SAFETY: the host library's function of that name, which takes a window.
            let f = unsafe { std::mem::transmute::<usize, extern "C" fn(u64) -> u64>(f) };
            f(w)
        }
        None => {
            // SAFETY: the host library's function of that name, which takes nothing.
            let f = unsafe { std::mem::transmute::<usize, extern "C" fn() -> u64>(f) };
            f()
        }
    }
}

/// Names, when it changes, what SDL holds about focus and the cursor: called
/// on the thread that takes the events.
fn note_focus_state() {
    let window = WINDOW.load(std::sync::atomic::Ordering::Relaxed);
    if window == 0 {
        return;
    }
    let state = [
        window_state("SDL_GetKeyboardFocus", None),
        window_state("SDL_GetMouseFocus", None),
        window_state("SDL_GetWindowFlags", Some(window)),
        window_state("SDL_CursorVisible", None) & 0xff,
        window_state("SDL_GetWindowRelativeMouseMode", Some(window)) & 0xff,
        window_state("SDL_TextInputActive", Some(window)) & 0xff,
    ];
    static LAST: Mutex<[u64; 6]> = Mutex::new([u64::MAX; 6]);
    let Ok(mut last) = LAST.lock() else {
        return;
    };
    if *last != state {
        *last = state;
        eprintln!(
            "cs2metal: SDL state: window {window:#x}, keyboard focus {:#x}, mouse focus {:#x}, \
             flags {:#x}, cursor visible {}, relative mouse {}, text input {}",
            state[0], state[1], state[2], state[3], state[4], state[5]
        );
    }
}

/// The type of the SDL_Event at `event`.
///
/// # Safety
///
/// `event` must point at an SDL_Event.
unsafe fn event_kind(event: u64) -> u32 {
    // SAFETY: the caller's contract; every event starts with its type.
    unsafe { std::ptr::read_unaligned(event as *const u32) }
}

/// `SDL_PollEvent(event)`, traced.
extern "C" fn traced_poll_event(event: u64) -> bool {
    let f = real("SDL_PollEvent");
    if f == 0 {
        return false;
    }
    // SAFETY: the host library's SDL_PollEvent.
    let f = unsafe { std::mem::transmute::<usize, extern "C" fn(u64) -> bool>(f) };
    let got = f(event);
    if got && event != 0 {
        // SAFETY: SDL filled the guest's event.
        note_event("SDL_PollEvent", unsafe { event_kind(event) });
    }
    note_focus_state();
    got
}

/// `SDL_WaitEventTimeout(event, timeout)`, traced.
extern "C" fn traced_wait_event_timeout(event: u64, timeout: i32) -> bool {
    let f = real("SDL_WaitEventTimeout");
    if f == 0 {
        return false;
    }
    // SAFETY: the host library's SDL_WaitEventTimeout.
    let f = unsafe { std::mem::transmute::<usize, extern "C" fn(u64, i32) -> bool>(f) };
    let got = f(event, timeout);
    if got && event != 0 {
        // SAFETY: SDL filled the guest's event.
        note_event("SDL_WaitEventTimeout", unsafe { event_kind(event) });
    }
    got
}

/// Set by SIGTERM or SIGINT: the game is to be asked to quit.
static QUIT_ASKED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// The handler of SIGTERM and SIGINT. It may run on any thread, with GS at the
/// guest's control block: it sets a flag and puts the default action back, so
/// that a second signal ends a game that does not quit.
extern "C" fn ask_to_quit(signal: libc::c_int) {
    QUIT_ASKED.store(true, std::sync::atomic::Ordering::Relaxed);
    // SAFETY: restores the default action of the signal being handled.
    unsafe { libc::signal(signal, libc::SIG_DFL) };
}

/// Makes SIGTERM and SIGINT ask the game to quit, as closing its window would,
/// instead of ending the process where it stands. A process ended in the middle
/// of its work leaves what it was writing as it was: Metal's shader cache among
/// it, which the next start then compiles again.
fn quit_on_signals() {
    for signal in [libc::SIGTERM, libc::SIGINT] {
        // SAFETY: a zeroed sigaction with a handler of the plain form.
        unsafe {
            let mut action: libc::sigaction = std::mem::zeroed();
            action.sa_sigaction = ask_to_quit as *const () as usize;
            action.sa_flags = libc::SA_RESTART;
            libc::sigemptyset(&mut action.sa_mask);
            libc::sigaction(signal, &action, std::ptr::null_mut());
        }
    }
}

/// `SDL_EVENT_QUIT`.
const EVENT_QUIT: u32 = 0x100;

/// Pushes SDL's quit event if a signal asked for it, once per signal: on the
/// thread that takes the game's events.
fn quit_if_asked() {
    if !QUIT_ASKED.swap(false, std::sync::atomic::Ordering::Relaxed) {
        return;
    }
    let push = real("SDL_PushEvent");
    if push == 0 {
        return;
    }
    // An SDL_Event is 128 bytes and starts with its type; SDL stamps a zero time.
    let mut event = [0u32; 32];
    event[0] = EVENT_QUIT;
    // SAFETY: the host library's SDL_PushEvent, which copies the event.
    let push = unsafe { std::mem::transmute::<usize, extern "C" fn(*mut [u32; 32]) -> bool>(push) };
    push(&mut event);
    eprintln!("cs2metal: a signal asked the game to quit");
}

/// `SDL_PeepEvents(events, count, action, min, max)`: the game takes its
/// events with it, on its main thread, several times a frame. That is where
/// the window is looked at again for [`fit_screen`], a few times a second, so
/// that whatever way its size or border changed, the Dock and the menu bar
/// follow. With the trace on, the events handed out are counted; an SDL_Event
/// is 128 bytes.
extern "C" fn peep_events(events: u64, count: i32, action: u32, min: u32, max: u32) -> i32 {
    static REAL: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let mut f = REAL.load(std::sync::atomic::Ordering::Relaxed);
    if f == 0 {
        f = real("SDL_PeepEvents");
        REAL.store(f, std::sync::atomic::Ordering::Relaxed);
    }
    if f == 0 {
        return -1;
    }
    // SAFETY: the host library's SDL_PeepEvents.
    let f =
        unsafe { std::mem::transmute::<usize, extern "C" fn(u64, i32, u32, u32, u32) -> i32>(f) };
    quit_if_asked();
    let got = f(events, count, action, min, max);
    fit_screen_in_time();
    if !tracing() {
        return got;
    }
    note_focus_state();
    // SDL_ADDEVENT is 0; the other actions hand events out.
    if action != 0 && events != 0 {
        for i in 0..u64::try_from(got).unwrap_or(0) {
            // SAFETY: SDL filled `got` of the guest's events.
            note_event("SDL_PeepEvents", unsafe { event_kind(events + 128 * i) });
        }
    }
    got
}

/// A wrapper that names every call of the host library's `$name` with its
/// first two arguments and its result, for the trace: calls that are few.
macro_rules! named {
    ($wrapper:ident, $name:literal) => {
        extern "C" fn $wrapper(a: u64, b: u64, c: u64) -> u64 {
            let f = real($name);
            if f == 0 {
                return 0;
            }
            // SAFETY: the host library's function of that name; the ABI
            // ignores arguments it does not take.
            let f = unsafe { std::mem::transmute::<usize, extern "C" fn(u64, u64, u64) -> u64>(f) };
            let result = f(a, b, c);
            let callers = guest_callers();
            eprintln!(
                "cs2metal: {}({a:#x}, {:#x}) returned {:#x}, callers {:#x} {:#x} {:#x}",
                $name,
                b & 0xffff_ffff,
                result & 0xff,
                callers[0],
                callers[1],
                callers[2]
            );
            result
        }
    };
}

/// The guest return addresses above a wrapper the guest called through the
/// host thunk, for the trace: where the guest made the call, from the thunk's
/// own record, then that function's callers. The thunk leaves `rbp` alone, so
/// the wrapper's saved `rbp` is the guest caller's frame.
#[inline(always)]
fn guest_callers() -> [u64; 3] {
    let mut found = [0u64; 3];
    found[0] = crate::syscall::BLOCK.with(|b| {
        b.try_borrow()
            .ok()
            .and_then(|b| b.as_ref().and_then(|b| b.current_call()))
            .map_or(0, |(_, back)| back)
    });
    #[cfg(target_arch = "x86_64")]
    {
        let mut frame: u64;
        // SAFETY: reads a register.
        unsafe { core::arch::asm!("mov {}, rbp", out(reg) frame, options(nomem, nostack)) };
        // SAFETY: this function's own frame record.
        frame = unsafe { std::ptr::read(frame as *const u64) };
        for slot in &mut found[1..] {
            if frame == 0 || frame % 8 != 0 {
                break;
            }
            // SAFETY: a frame record of the guest's stack, by the guest's
            // frame pointers.
            let [next, caller] = unsafe { std::ptr::read(frame as *const [u64; 2]) };
            *slot = caller;
            if next <= frame {
                break;
            }
            frame = next;
        }
    }
    found
}

/// A wrapper that names a call of the host library's `$name` whenever its
/// first argument or its result differs from the call before, for the trace:
/// lookups that are made often and seldom change.
macro_rules! changed {
    ($wrapper:ident, $name:literal) => {
        extern "C" fn $wrapper(a: u64, b: u64, c: u64) -> u64 {
            let f = real($name);
            if f == 0 {
                return 0;
            }
            // SAFETY: the host library's function of that name; the ABI
            // ignores arguments it does not take.
            let f = unsafe { std::mem::transmute::<usize, extern "C" fn(u64, u64, u64) -> u64>(f) };
            let result = f(a, b, c);
            static LAST: Mutex<(u64, u64)> = Mutex::new((u64::MAX, u64::MAX));
            if let Ok(mut last) = LAST.lock() {
                if *last != (a, result) {
                    *last = (a, result);
                    eprintln!("cs2metal: {}({a:#x}) returned {result:#x}", $name);
                }
            }
            result
        }
    };
}

changed!(changed_window_from_id, "SDL_GetWindowFromID");
changed!(changed_window_id, "SDL_GetWindowID");
changed!(changed_mouse_focus, "SDL_GetMouseFocus");
changed!(changed_grabbed_window, "SDL_GetGrabbedWindow");
changed!(changed_window_flags, "SDL_GetWindowFlags");

named!(named_relative_mouse, "SDL_SetWindowRelativeMouseMode");
named!(named_mouse_grab, "SDL_SetWindowMouseGrab");
named!(named_start_text, "SDL_StartTextInput");
named!(named_stop_text, "SDL_StopTextInput");
named!(named_raise_window, "SDL_RaiseWindow");
named!(named_show_cursor, "SDL_ShowCursor");
named!(named_hide_cursor, "SDL_HideCursor");

/// `SDL_SetHint(name, value)`, traced.
extern "C" fn traced_set_hint(name: *const c_char, value: *const c_char) -> bool {
    let f = real("SDL_SetHint");
    if f == 0 {
        return false;
    }
    let text = |p: *const c_char| {
        if p.is_null() {
            "null".to_owned()
        } else {
            // SAFETY: the guest passes C strings.
            unsafe { CStr::from_ptr(p) }.to_string_lossy().into_owned()
        }
    };
    eprintln!("cs2metal: SDL_SetHint({}, {})", text(name), text(value));
    // SAFETY: the host library's SDL_SetHint.
    let f = unsafe {
        std::mem::transmute::<usize, extern "C" fn(*const c_char, *const c_char) -> bool>(f)
    };
    f(name, value)
}

/// `SDL_GetKeyboardFocus()`, traced: each change of the window that has it.
extern "C" fn traced_keyboard_focus() -> u64 {
    let f = real("SDL_GetKeyboardFocus");
    if f == 0 {
        return 0;
    }
    // SAFETY: the host library's SDL_GetKeyboardFocus.
    let f = unsafe { std::mem::transmute::<usize, extern "C" fn() -> u64>(f) };
    let window = f();
    static LAST: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(u64::MAX);
    if LAST.swap(window, std::sync::atomic::Ordering::Relaxed) != window {
        eprintln!("cs2metal: SDL_GetKeyboardFocus is now {window:#x}");
    }
    window
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

/// The private system call the shim's callback thread makes for its next call.
pub(crate) const NEXT_CALLBACK: u64 = 0x4353_0004;

/// A stream opened with a guest callback, which host SDL would call on its own
/// audio thread. That thread runs no guest thread, and running the callback on
/// another while SDL holds the stream's lock would deadlock, so the stream is
/// opened without it, and the shim's callback thread makes the calls SDL's
/// device thread would: whenever the queue falls below a target, with the
/// stream locked, as SDL calls a stream's callback.
struct Pump {
    stream: u64,
    callback: u64,
    userdata: u64,
    /// Bytes per second of the stream's input side, and per frame.
    rate: u64,
    frame: u64,
    playback: bool,
}

struct Pumps {
    list: Vec<Pump>,
    /// The stream the callback thread holds locked while the guest's callback
    /// runs, or 0.
    held: u64,
    /// When the last call was handed out.
    last: Option<std::time::Instant>,
}

static PUMPS: Mutex<Pumps> = Mutex::new(Pumps {
    list: Vec::new(),
    held: 0,
    last: None,
});
static PUMPS_CHANGED: std::sync::Condvar = std::sync::Condvar::new();

/// The guest thread id of the shim's callback thread, once it has asked for a
/// call.
static CALLBACK_THREAD: std::sync::atomic::AtomicI64 = std::sync::atomic::AtomicI64::new(0);

/// How long `SDL_DestroyAudioStream` waits for a callback on the stream to end.
const DESTROY_WAIT: std::time::Duration = std::time::Duration::from_secs(2);

fn tracing() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("CS2METAL_SDL_TRACE").is_some())
}

/// `SDL_LockAudioStream(stream)` or `SDL_UnlockAudioStream(stream)`, on the
/// calling thread.
fn lock_stream(stream: u64, lock: bool) {
    let f = real(if lock {
        "SDL_LockAudioStream"
    } else {
        "SDL_UnlockAudioStream"
    });
    if f != 0 {
        // SAFETY: the host library's function of that name.
        let f: extern "C" fn(u64) -> bool = unsafe { std::mem::transmute(f) };
        f(stream);
    }
}

/// How far ahead of the device a playback stream's queue is kept.
const AHEAD_MS: u64 = 40;
/// How often the queues are looked at.
const TICK: std::time::Duration = std::time::Duration::from_millis(5);
/// The least time between two calls handed out. SDL's device thread calls a
/// stream's callback once for each buffer the device takes, about every 20 ms.
/// The game's callback does not always queue what it is asked for: it keeps its
/// own, smaller, reserve. Asked again at once, it was called in a loop that
/// never slept, which kept one core busy and the stream's lock contended.
const PACE: std::time::Duration = std::time::Duration::from_millis(10);

const DEFAULT_RECORDING_DEVICE: u32 = 0xffff_fffe;

/// `SDL_OpenAudioDeviceStream(device, spec, callback, userdata)`.
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
    // SAFETY: the host library's SDL_OpenAudioDeviceStream.
    let f: extern "C" fn(u32, u64, u64, u64) -> u64 = unsafe { std::mem::transmute(f) };
    let stream = f(device, spec, 0, 0);
    if stream != 0 && tracing() {
        trace_device_mix(stream);
    }
    if stream == 0 || callback == 0 || spec == 0 {
        return stream;
    }
    // SDL_AudioSpec: format (its low byte the sample's bits), channels, rate.
    // SAFETY: the guest passes an SDL_AudioSpec.
    let [format, channels, freq] = unsafe { std::ptr::read_unaligned(spec as *const [u32; 3]) };
    let frame = u64::from((format & 0xff) / 8) * u64::from(channels.max(1));
    if tracing() {
        eprintln!(
            "cs2metal: SDL audio stream with a callback: format {format:#x}, {channels} channels, \
             {freq} Hz, on device {device:#x} ({})",
            device_name(device)
        );
    }
    if let Ok(mut pumps) = PUMPS.lock() {
        pumps.list.push(Pump {
            stream,
            callback,
            userdata,
            rate: frame * u64::from(freq),
            frame: frame.max(1),
            playback: device != DEFAULT_RECORDING_DEVICE,
        });
        PUMPS_CHANGED.notify_all();
    }
    stream
}

/// A wrapper that counts the calls of the host library's `$name`, which takes
/// at most three integer arguments, for the trace.
macro_rules! counted {
    ($wrapper:ident, $name:literal) => {
        extern "C" fn $wrapper(a: u64, b: u64, c: u64) -> u64 {
            static CALLS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
            let n = CALLS.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
            if n.is_power_of_two() {
                eprintln!("cs2metal: {} (call {n})", $name);
            }
            let f = real($name);
            if f == 0 {
                return 0;
            }
            // SAFETY: the host library's function of that name; the ABI
            // ignores arguments it does not take.
            let f = unsafe { std::mem::transmute::<usize, extern "C" fn(u64, u64, u64) -> u64>(f) };
            f(a, b, c)
        }
    };
}

counted!(counted_clear, "SDL_ClearAudioStream");
counted!(counted_flush, "SDL_FlushAudioStream");
counted!(counted_pause, "SDL_PauseAudioDevice");
counted!(counted_resume, "SDL_ResumeAudioDevice");
counted!(counted_get, "SDL_GetAudioStreamData");

/// Host SDL's postmix callback, for the trace: the loudest sample of what the
/// device is about to play, on SDL's device thread.
extern "C" fn postmix(_userdata: u64, _spec: u64, buffer: u64, len: i32) {
    static CALLS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let n = CALLS.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
    if n.is_power_of_two() && buffer != 0 && len > 0 {
        // SAFETY: SDL passes `len` bytes of float samples.
        let peak = unsafe { peak(buffer, len as usize) };
        eprintln!("cs2metal: SDL device mix of {len} bytes, peak {peak:.4} (call {n})");
    }
}

/// Watches what the device `stream` is bound to plays, for the trace.
fn trace_device_mix(stream: u64) {
    let (device, set) = (
        real("SDL_GetAudioStreamDevice"),
        real("SDL_SetAudioPostmixCallback"),
    );
    if device == 0 || set == 0 {
        return;
    }
    // SAFETY: the host library's functions of these names.
    let (device, set) = unsafe {
        (
            std::mem::transmute::<usize, extern "C" fn(u64) -> u32>(device),
            std::mem::transmute::<usize, extern "C" fn(u32, u64, u64) -> bool>(set),
        )
    };
    set(device(stream), postmix as *const () as u64, 0);
}

/// What host SDL calls the audio device `device`, for the trace.
fn device_name(device: u32) -> String {
    let f = real("SDL_GetAudioDeviceName");
    if f == 0 {
        return String::new();
    }
    // SAFETY: the host library's SDL_GetAudioDeviceName.
    let f: extern "C" fn(u32) -> *const c_char = unsafe { std::mem::transmute(f) };
    let name = f(device);
    if name.is_null() {
        return "no name".to_owned();
    }
    // SAFETY: SDL returns a C string.
    unsafe { CStr::from_ptr(name) }
        .to_string_lossy()
        .into_owned()
}

/// The loudest sample in `len` bytes of 32-bit float audio at `data`.
///
/// # Safety
///
/// `data` must point at `len` readable bytes.
unsafe fn peak(data: u64, len: usize) -> f32 {
    (0..len / 4)
        // SAFETY: the caller's contract.
        .map(|i| unsafe { std::ptr::read_unaligned((data as *const f32).add(i)) }.abs())
        .fold(0.0, f32::max)
}

/// `SDL_PutAudioStreamData(stream, data, len)`, traced: how much the game
/// queues, and how loud, which tells silence queued from nothing queued.
extern "C" fn put_audio_stream_data(stream: u64, data: u64, len: i32) -> bool {
    let f = real("SDL_PutAudioStreamData");
    if f == 0 {
        return false;
    }
    static CALLS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let n = CALLS.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
    if n.is_power_of_two() && data != 0 && len > 0 {
        // SAFETY: the guest passes `len` bytes; read as the float samples the
        // game's stream holds.
        let peak = unsafe { peak(data, len as usize) };
        eprintln!("cs2metal: SDL_PutAudioStreamData of {len} bytes, peak {peak:.4} (call {n})");
    }
    // SAFETY: the host library's SDL_PutAudioStreamData.
    let f: extern "C" fn(u64, u64, i32) -> bool = unsafe { std::mem::transmute(f) };
    f(stream, data, len)
}

/// `SDL_DestroyAudioStream(stream)`: its pump goes first, under the lock the
/// callback thread takes, and a callback running on the stream ends first. A
/// callback that destroys its own stream gives its lock up here.
extern "C" fn destroy_audio_stream(stream: u64) {
    let f = real("SDL_DestroyAudioStream");
    let Ok(mut pumps) = PUMPS.lock() else {
        return;
    };
    pumps.list.retain(|p| p.stream != stream);
    let own = CALLBACK_THREAD.load(std::sync::atomic::Ordering::Acquire);
    let _pumps = if pumps.held == stream && own == crate::threads::gettid() {
        pumps.held = 0;
        lock_stream(stream, false);
        pumps
    } else {
        match PUMPS_CHANGED.wait_timeout_while(pumps, DESTROY_WAIT, |p| p.held == stream) {
            Ok((p, _)) => p,
            Err(_) => return,
        }
    };
    if f != 0 {
        // SAFETY: the host library's SDL_DestroyAudioStream.
        let f: extern "C" fn(u64) = unsafe { std::mem::transmute(f) };
        f(stream);
    }
}

/// Whether the device `stream` is bound to is paused. A device opened with
/// `SDL_OpenAudioDeviceStream` starts paused, and SDL calls no callback until
/// the game resumes it; the game finishes setting its side up in between.
fn paused(stream: u64) -> bool {
    let (device, paused) = (
        real("SDL_GetAudioStreamDevice"),
        real("SDL_AudioDevicePaused"),
    );
    if device == 0 || paused == 0 {
        return false;
    }
    // SAFETY: the host library's functions of these names.
    let (device, paused) = unsafe {
        (
            std::mem::transmute::<usize, extern "C" fn(u64) -> u32>(device),
            std::mem::transmute::<usize, extern "C" fn(u32) -> bool>(paused),
        )
    };
    paused(device(stream))
}

/// Counts the times a playing stream's queue was found empty, which the device
/// plays as silence, and names them in the trace.
fn ran_dry() {
    static TIMES: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let n = TIMES.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
    if tracing() && n.is_power_of_two() {
        eprintln!("cs2metal: SDL audio queue found empty ({n} times)");
    }
}

/// The next stream callback due, as (callback, userdata, stream, amount), among
/// the streams whose devices are playing.
fn due(pumps: &[Pump]) -> Option<[u64; 5]> {
    let queued = real("SDL_GetAudioStreamQueued");
    let available = real("SDL_GetAudioStreamAvailable");
    if queued == 0 || available == 0 {
        return None;
    }
    // SAFETY: the host library's functions of these names.
    let (queued, available): (extern "C" fn(u64) -> i32, extern "C" fn(u64) -> i32) = unsafe {
        (
            std::mem::transmute::<usize, extern "C" fn(u64) -> i32>(queued),
            std::mem::transmute::<usize, extern "C" fn(u64) -> i32>(available),
        )
    };
    pumps.iter().filter(|p| !paused(p.stream)).find_map(|p| {
        let amount = if p.playback {
            let target = p.rate * AHEAD_MS / 1000;
            let have = u64::try_from(queued(p.stream)).unwrap_or(0);
            if have == 0 {
                ran_dry();
            }
            target.saturating_sub(have) / p.frame * p.frame
        } else {
            u64::try_from(available(p.stream)).unwrap_or(0)
        };
        (amount > 0).then_some([p.callback, p.userdata, p.stream, amount, amount])
    })
}

/// The shim's callback thread asks for its next call: it waits until a stream
/// needs data and writes the call, a guest function and four arguments, to
/// `out`.
///
/// SDL calls a stream's callback with the stream locked, and a callback may
/// count on that: the game's unlocks the stream while it mixes and locks it
/// again before it returns. So the stream is locked here, on the callback
/// thread, before the call is handed out, and unlocked when the thread comes
/// back for the next one. Without that the game's own lock was never given
/// up, and SDL's device thread waited for it forever, in silence.
pub(crate) fn next_callback(out: u64) -> i64 {
    if out == 0 {
        return crate::translate::EFAULT;
    }
    CALLBACK_THREAD.store(
        crate::threads::gettid(),
        std::sync::atomic::Ordering::Release,
    );
    let Ok(mut pumps) = PUMPS.lock() else {
        return crate::translate::EINVAL;
    };
    let held = std::mem::take(&mut pumps.held);
    if held != 0 {
        lock_stream(held, false);
        PUMPS_CHANGED.notify_all();
    }
    let call = loop {
        let rest = pumps.last.map_or(std::time::Duration::ZERO, |last| {
            PACE.saturating_sub(last.elapsed())
        });
        if rest.is_zero() {
            if let Some(call) = due(&pumps.list) {
                break call;
            }
        }
        pumps = match PUMPS_CHANGED.wait_timeout(pumps, rest.max(TICK)) {
            Ok((p, _)) => p,
            Err(_) => return crate::translate::EINVAL,
        };
    };
    pumps.last = Some(std::time::Instant::now());
    // Marked held before the stream is locked, outside the pumps' lock: its
    // destruction waits meanwhile, and a thread that holds the stream's lock
    // can still reach the pumps.
    pumps.held = call[2];
    drop(pumps);
    lock_stream(call[2], true);
    if tracing() {
        static CALLS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let n = CALLS.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
        if n.is_power_of_two() {
            eprintln!(
                "cs2metal: SDL audio callback for {} bytes (call {n})",
                call[3]
            );
        }
    }
    // SAFETY: the shim passes room for five words.
    unsafe { std::ptr::write_unaligned(out as *mut [u64; 5], call) };
    0
}

// --- Windows -----------------------------------------------------------------------

/// `SDL_MinimizeWindow(window)`: not done. The game minimizes its window when it
/// loses the focus, as a Linux game may; a process started from a terminal starts
/// without the focus, so the window went to the Dock at once. On macOS a minimized
/// window has no drawable, and the renderer then stops on it.
extern "C" fn minimize_window(_window: u64) -> bool {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| eprintln!("cs2metal: the game asked to minimize its window; kept up"));
    true
}

// --- The screen ----------------------------------------------------------------------

#[link(name = "objc")]
unsafe extern "C" {
    fn objc_getClass(name: *const c_char) -> *mut c_void;
    fn sel_registerName(name: *const c_char) -> *mut c_void;
    fn objc_msgSend();
}

/// `NSApplicationPresentationHideDock | NSApplicationPresentationHideMenuBar`.
const HIDE_DOCK_AND_MENU_BAR: usize = (1 << 1) | (1 << 3);

/// `[[NSApplication sharedApplication] setPresentationOptions:options]`, on the
/// main thread only, where AppKit allows it.
fn set_presentation(options: usize) {
    // SAFETY: pthread_main_np has no preconditions.
    if unsafe { libc::pthread_main_np() } == 0 {
        return;
    }
    // SAFETY: the Objective-C runtime's lookups by name, and two messages with
    // the signatures AppKit declares for them: one returns the application
    // object, the other takes an unsigned word. Both option sets used here are
    // ones AppKit accepts.
    unsafe {
        let class = objc_getClass(c"NSApplication".as_ptr());
        if class.is_null() {
            return;
        }
        let send = objc_msgSend as *const ();
        let shared = std::mem::transmute::<
            *const (),
            extern "C" fn(*mut c_void, *mut c_void) -> *mut c_void,
        >(send);
        let application = shared(class, sel_registerName(c"sharedApplication".as_ptr()));
        if application.is_null() {
            return;
        }
        let set =
            std::mem::transmute::<*const (), extern "C" fn(*mut c_void, *mut c_void, usize)>(send);
        set(
            application,
            sel_registerName(c"setPresentationOptions:".as_ptr()),
            options,
        );
    }
}

const WINDOW_FULLSCREEN: u64 = 0x1;
const WINDOW_HIDDEN: u64 = 0x8;
const WINDOW_BORDERLESS: u64 = 0x10;
const WINDOW_MINIMIZED: u64 = 0x40;

/// Whether a window with these flags and this size takes its whole display: it
/// is shown, and either SDL holds it full screen or it has no border and is as
/// large as the display, which is the game's "full screen window".
fn covers_display(flags: u64, size: (i32, i32), display: (i32, i32)) -> bool {
    let borderless_and_as_large = flags & WINDOW_BORDERLESS != 0 && size.0 > 0 && size == display;
    flags & (WINDOW_HIDDEN | WINDOW_MINIMIZED) == 0
        && (flags & WINDOW_FULLSCREEN != 0 || borderless_and_as_large)
}

/// Whether the Dock and the menu bar are hidden for the game's window.
static SCREEN_TAKEN: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Hides the Dock and the menu bar while `window` is a borderless window that
/// covers its display, and shows them again when it no longer is.
///
/// The game's "full screen window" mode makes such a window and asks SDL for
/// nothing more. On Linux the window manager treats it as full screen. On
/// macOS the Dock and the menu bar stay in front of it, unless the application
/// asks for them to be hidden while it is active.
fn fit_screen(window: u64) {
    let (flags, size, display, bounds) = (
        real("SDL_GetWindowFlags"),
        real("SDL_GetWindowSize"),
        real("SDL_GetDisplayForWindow"),
        real("SDL_GetDisplayBounds"),
    );
    if window == 0 || flags == 0 || size == 0 || display == 0 || bounds == 0 {
        return;
    }
    // SAFETY: the host library's functions of these names.
    let (flags, size, display, bounds) = unsafe {
        (
            std::mem::transmute::<usize, extern "C" fn(u64) -> u64>(flags),
            std::mem::transmute::<usize, extern "C" fn(u64, *mut i32, *mut i32) -> bool>(size),
            std::mem::transmute::<usize, extern "C" fn(u64) -> u32>(display),
            std::mem::transmute::<usize, extern "C" fn(u32, *mut [i32; 4]) -> bool>(bounds),
        )
    };
    let (mut width, mut height, mut rect) = (0, 0, [0i32; 4]);
    let known = size(window, &mut width, &mut height) && bounds(display(window), &mut rect);
    let flags = flags(window);
    let taken = known && covers_display(flags, (width, height), (rect[2], rect[3]));
    if SCREEN_TAKEN.swap(taken, std::sync::atomic::Ordering::Relaxed) != taken {
        set_presentation(if taken { HIDE_DOCK_AND_MENU_BAR } else { 0 });
        if tracing() {
            eprintln!(
                "cs2metal: the Dock and the menu bar are {} for a {width}x{height} window with \
                 flags {flags:#x} on a {}x{} display",
                if taken { "hidden" } else { "shown" },
                rect[2],
                rect[3]
            );
        }
    }
}

/// [`fit_screen`] for the game's window, at most four times a second.
fn fit_screen_in_time() {
    const EVERY_MS: u64 = 250;
    static START: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
    static NEXT_MS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let now = START
        .get_or_init(std::time::Instant::now)
        .elapsed()
        .as_millis() as u64;
    if now >= NEXT_MS.load(std::sync::atomic::Ordering::Relaxed) {
        NEXT_MS.store(now + EVERY_MS, std::sync::atomic::Ordering::Relaxed);
        fit_screen(WINDOW.load(std::sync::atomic::Ordering::Relaxed));
    }
}

/// A wrapper for a host function that takes the window and up to two more
/// integer arguments and may change the window's size, border, or state: the
/// call, then [`fit_screen`].
macro_rules! fitted {
    ($wrapper:ident, $name:literal) => {
        extern "C" fn $wrapper(window: u64, b: u64, c: u64) -> u64 {
            let f = real($name);
            if f == 0 {
                return 0;
            }
            // SAFETY: the host library's function of that name; the ABI
            // ignores arguments it does not take.
            let f = unsafe { std::mem::transmute::<usize, extern "C" fn(u64, u64, u64) -> u64>(f) };
            let result = f(window, b, c);
            fit_screen(window);
            result
        }
    };
}

fitted!(fitted_set_size, "SDL_SetWindowSize");
fitted!(fitted_set_position, "SDL_SetWindowPosition");
fitted!(fitted_set_bordered, "SDL_SetWindowBordered");
fitted!(fitted_set_fullscreen, "SDL_SetWindowFullscreen");
fitted!(fitted_set_fullscreen_mode, "SDL_SetWindowFullscreenMode");
fitted!(fitted_sync, "SDL_SyncWindow");
fitted!(fitted_restore, "SDL_RestoreWindow");

/// `SDL_DestroyWindow(window)`: the Dock and the menu bar come back first.
extern "C" fn destroy_window(window: u64) {
    if SCREEN_TAKEN.swap(false, std::sync::atomic::Ordering::Relaxed) {
        set_presentation(0);
    }
    let f = real("SDL_DestroyWindow");
    if f != 0 {
        // SAFETY: the host library's SDL_DestroyWindow.
        let f = unsafe { std::mem::transmute::<usize, extern "C" fn(u64)>(f) };
        f(window);
    }
}

/// The window property the game's tier-0 module reads as a window's operating
/// system handle, unless SDL's video driver is Wayland.
const X11_WINDOW: &CStr = c"SDL.window.x11.window";

/// Gives `window` the X11 window number the game looks for: its SDL window id.
///
/// The game keeps its windows by that handle. Its input system asks, for each
/// handle it was given, for the window again, and then for relative mouse mode
/// or text input on it. Host SDL's video driver is Cocoa, which sets no such
/// property: the handle read as zero, the window as null, and those calls
/// failed, so the pointer stayed visible and free in a match and nothing could
/// be typed into the game's console. Nothing else in the game reads the
/// number, and nothing talks to an X server with it.
fn name_window(window: u64) {
    let (properties, id, set) = (
        real("SDL_GetWindowProperties"),
        real("SDL_GetWindowID"),
        real("SDL_SetNumberProperty"),
    );
    if window == 0 || properties == 0 || id == 0 || set == 0 {
        return;
    }
    // SAFETY: the host library's functions of these names.
    let (properties, id, set) = unsafe {
        (
            std::mem::transmute::<usize, extern "C" fn(u64) -> u32>(properties),
            std::mem::transmute::<usize, extern "C" fn(u64) -> u32>(id),
            std::mem::transmute::<usize, extern "C" fn(u32, *const c_char, i64) -> bool>(set),
        )
    };
    let named = set(
        properties(window),
        X11_WINDOW.as_ptr(),
        i64::from(id(window)),
    );
    if !named {
        eprintln!("cs2metal: the game's window could not be given its X11 number");
    }
}

/// `SDL_CreateWindowWithProperties(props)`: the window, named as the game will
/// look for it, then raised and given the focus, since the process does not come
/// to the front by itself.
extern "C" fn create_window_with_properties(props: u32) -> u64 {
    let f = real("SDL_CreateWindowWithProperties");
    if f == 0 {
        return 0;
    }
    // SAFETY: the host library's SDL_CreateWindowWithProperties.
    let f: extern "C" fn(u32) -> u64 = unsafe { std::mem::transmute(f) };
    let window = f(props);
    name_window(window);
    WINDOW.store(window, std::sync::atomic::Ordering::Relaxed);
    let raise = real("SDL_RaiseWindow");
    if window != 0 && raise != 0 {
        // SAFETY: the host library's SDL_RaiseWindow, on the window just made.
        let raise: extern "C" fn(u64) -> bool = unsafe { std::mem::transmute(raise) };
        raise(window);
    }
    fit_screen(window);
    window
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

/// Keeps the game's window up when another application takes the focus, unless
/// the owner sets the hint. The game asks SDL to minimize it, as a Linux game
/// may; on macOS a minimized window has no drawable, the renderer then waits for
/// one on the main thread, and the main thread stops handling the events that
/// would restore the window. An environment variable takes precedence over the
/// game's own `SDL_SetHint`.
fn keep_window_on_focus_loss() {
    const HINT: &str = "SDL_VIDEO_MINIMIZE_ON_FOCUS_LOSS";
    if std::env::var_os(HINT).is_none() {
        // SAFETY: the host environment; the guest's is separate.
        unsafe { std::env::set_var(HINT, "0") };
    }
}

/// Keeps host SDL from putting the game's window in a macOS full-screen Space,
/// unless the owner sets the hint. A Space is entered and left with an
/// animation, behind SDL's back as far as the game can tell: changing the
/// display mode left the window with a title bar while SDL reported it full
/// screen, or full size with the Dock in front. Without Spaces SDL's full
/// screen is a window over the whole display, at once.
fn no_fullscreen_spaces() {
    const HINT: &str = "SDL_VIDEO_MAC_FULLSCREEN_SPACES";
    if std::env::var_os(HINT).is_none() {
        // SAFETY: the host environment; the guest's is separate.
        unsafe { std::env::set_var(HINT, "0") };
    }
}

/// Points the macOS Vulkan loader at the SDK's MoltenVK manifest, in the host
/// environment, unless the owner names drivers.
pub(crate) fn prepare_vulkan_environment() {
    // MoltenVK compiles each shader with Metal's compiler, by default one at a
    // time. The renderer compiles pipelines on several threads, and a pipeline
    // seen for the first time stalls a frame until its shaders are compiled.
    const CONCURRENT: &str = "MVK_CONFIG_SHOULD_MAXIMIZE_CONCURRENT_COMPILATION";
    if std::env::var_os(CONCURRENT).is_none() {
        // SAFETY: the host environment; the guest's is separate.
        unsafe { std::env::set_var(CONCURRENT, "1") };
    }
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
        b"SDL_DestroyAudioStream" => destroy_audio_stream as *const (),
        b"SDL_PollEvent" if tracing() => traced_poll_event as *const (),
        b"SDL_WaitEventTimeout" if tracing() => traced_wait_event_timeout as *const (),
        b"SDL_GetWindowFromID" if tracing() => changed_window_from_id as *const (),
        b"SDL_GetWindowID" if tracing() => changed_window_id as *const (),
        b"SDL_GetMouseFocus" if tracing() => changed_mouse_focus as *const (),
        b"SDL_GetGrabbedWindow" if tracing() => changed_grabbed_window as *const (),
        b"SDL_GetWindowFlags" if tracing() => changed_window_flags as *const (),
        b"SDL_SetWindowRelativeMouseMode" if tracing() => named_relative_mouse as *const (),
        b"SDL_SetWindowMouseGrab" if tracing() => named_mouse_grab as *const (),
        b"SDL_StartTextInput" if tracing() => named_start_text as *const (),
        b"SDL_StopTextInput" if tracing() => named_stop_text as *const (),
        b"SDL_RaiseWindow" if tracing() => named_raise_window as *const (),
        b"SDL_ShowCursor" if tracing() => named_show_cursor as *const (),
        b"SDL_HideCursor" if tracing() => named_hide_cursor as *const (),
        b"SDL_SetHint" if tracing() => traced_set_hint as *const (),
        b"SDL_GetKeyboardFocus" if tracing() => traced_keyboard_focus as *const (),
        b"SDL_PutAudioStreamData" if tracing() => put_audio_stream_data as *const (),
        b"SDL_ClearAudioStream" if tracing() => counted_clear as *const (),
        b"SDL_FlushAudioStream" if tracing() => counted_flush as *const (),
        b"SDL_PauseAudioDevice" if tracing() => counted_pause as *const (),
        b"SDL_ResumeAudioDevice" if tracing() => counted_resume as *const (),
        b"SDL_GetAudioStreamData" if tracing() => counted_get as *const (),
        b"SDL_PeepEvents" => peep_events as *const (),
        b"SDL_MinimizeWindow" => minimize_window as *const (),
        b"SDL_SetWindowSize" => fitted_set_size as *const (),
        b"SDL_SetWindowPosition" => fitted_set_position as *const (),
        b"SDL_SetWindowBordered" => fitted_set_bordered as *const (),
        b"SDL_SetWindowFullscreen" => fitted_set_fullscreen as *const (),
        b"SDL_SetWindowFullscreenMode" => fitted_set_fullscreen_mode as *const (),
        b"SDL_SyncWindow" => fitted_sync as *const (),
        b"SDL_RestoreWindow" => fitted_restore as *const (),
        b"SDL_DestroyWindow" => destroy_window as *const (),
        b"SDL_CreateWindowWithProperties" => create_window_with_properties as *const (),
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
    keep_window_on_focus_loss();
    no_fullscreen_spaces();
    quit_on_signals();
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
    if !lacking.is_empty() && tracing() {
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

    #[cfg(target_arch = "x86_64")]
    extern "C" fn queued(_stream: u64) -> i32 {
        1000
    }

    #[cfg(target_arch = "x86_64")]
    extern "C" fn available(_stream: u64) -> i32 {
        0
    }

    /// A 48 kHz stereo float stream holding 1,000 bytes is due enough for 40 ms
    /// ahead, in whole frames.
    #[cfg(target_arch = "x86_64")]
    #[test]
    fn a_playback_stream_is_topped_up_in_whole_frames() {
        install_fakes();
        let pumps = [Pump {
            stream: 7,
            callback: 0x1000,
            userdata: 9,
            rate: 8 * 48_000,
            frame: 8,
            playback: true,
        }];
        let call = due(&pumps).expect("due");
        let target = 8 * 48_000 * AHEAD_MS / 1000;
        assert_eq!(call, [0x1000, 9, 7, target - 1000, target - 1000]);
        assert_eq!((target - 1000) % 8, 0);
    }

    /// A stream's device has the stream's number; device 13 is paused.
    #[cfg(target_arch = "x86_64")]
    extern "C" fn device(stream: u64) -> u32 {
        stream as u32
    }

    #[cfg(target_arch = "x86_64")]
    extern "C" fn device_paused(device: u32) -> bool {
        device == 13
    }

    /// A device starts paused, and SDL calls no stream callback until the game
    /// resumes it.
    #[cfg(target_arch = "x86_64")]
    #[test]
    fn a_paused_device_asks_for_nothing() {
        install_fakes();
        let pump = |stream| Pump {
            stream,
            callback: 0x1000,
            userdata: 9,
            rate: 8 * 48_000,
            frame: 8,
            playback: true,
        };
        assert!(due(&[pump(13)]).is_none());
        let call = due(&[pump(13), pump(7)]).expect("due");
        assert_eq!(call[2], 7);
    }

    #[cfg(target_arch = "x86_64")]
    static LOCKS: Mutex<Vec<(bool, u64)>> = Mutex::new(Vec::new());

    #[cfg(target_arch = "x86_64")]
    extern "C" fn lock(stream: u64) -> bool {
        LOCKS.lock().expect("locks").push((true, stream));
        true
    }

    #[cfg(target_arch = "x86_64")]
    extern "C" fn unlock(stream: u64) -> bool {
        LOCKS.lock().expect("locks").push((false, stream));
        true
    }

    /// The host library as these tests need it. Every test installs the same
    /// one, since they share the bridge's state.
    #[cfg(target_arch = "x86_64")]
    fn install_fakes() {
        let mut real = HashMap::new();
        for (name, f) in [
            ("SDL_GetAudioStreamQueued", queued as *const () as usize),
            (
                "SDL_GetAudioStreamAvailable",
                available as *const () as usize,
            ),
            (
                "SDL_GetWindowProperties",
                window_properties as *const () as usize,
            ),
            ("SDL_GetWindowID", window_id as *const () as usize),
            (
                "SDL_SetNumberProperty",
                set_number_property as *const () as usize,
            ),
            ("SDL_PushEvent", push_event as *const () as usize),
            ("SDL_GetAudioStreamDevice", device as *const () as usize),
            ("SDL_AudioDevicePaused", device_paused as *const () as usize),
            ("SDL_LockAudioStream", lock as *const () as usize),
            ("SDL_UnlockAudioStream", unlock as *const () as usize),
        ] {
            real.insert(name.to_owned(), f);
        }
        if let Ok(mut state) = STATE.lock() {
            *state = Some(State {
                bridge: Bridge::new().expect("bridge"),
                real,
                watches: HashMap::new(),
            });
        }
    }

    /// SDL calls a stream's callback with the stream locked: each call is handed
    /// out with the lock taken, and the lock is given up when the callback
    /// thread comes back for the next.
    #[cfg(target_arch = "x86_64")]
    #[test]
    fn a_callback_runs_with_its_stream_locked() {
        install_fakes();
        PUMPS.lock().expect("pumps").list.push(Pump {
            stream: 7,
            callback: 0x1000,
            userdata: 9,
            rate: 8 * 48_000,
            frame: 8,
            playback: true,
        });
        let mut call = [0u64; 5];
        let out = std::ptr::from_mut(&mut call) as u64;
        assert_eq!(next_callback(out), 0);
        assert_eq!(call[..3], [0x1000, 9, 7]);
        assert_eq!(*LOCKS.lock().expect("locks"), [(true, 7)]);
        assert_eq!(PUMPS.lock().expect("pumps").held, 7);
        let first = std::time::Instant::now();
        assert_eq!(next_callback(out), 0);
        assert!(
            first.elapsed() >= PACE - TICK,
            "the second call waits for the pace"
        );
        assert_eq!(
            *LOCKS.lock().expect("locks"),
            [(true, 7), (false, 7), (true, 7)]
        );
        // Destroyed from inside its own callback: the lock is given up first.
        destroy_audio_stream(7);
        assert_eq!(LOCKS.lock().expect("locks").last(), Some(&(false, 7)));
        let pumps = PUMPS.lock().expect("pumps");
        assert!(pumps.list.is_empty() && pumps.held == 0);
    }

    #[cfg(target_arch = "x86_64")]
    static NUMBERS: Mutex<Vec<(u32, String, i64)>> = Mutex::new(Vec::new());

    #[cfg(target_arch = "x86_64")]
    extern "C" fn window_properties(window: u64) -> u32 {
        window as u32 + 100
    }

    #[cfg(target_arch = "x86_64")]
    extern "C" fn window_id(window: u64) -> u32 {
        window as u32 + 1
    }

    #[cfg(target_arch = "x86_64")]
    extern "C" fn set_number_property(properties: u32, name: *const c_char, value: i64) -> bool {
        // SAFETY: the bridge passes a C string.
        let name = unsafe { CStr::from_ptr(name) }
            .to_string_lossy()
            .into_owned();
        NUMBERS
            .lock()
            .expect("numbers")
            .push((properties, name, value));
        true
    }

    /// The game finds its window again by the X11 window number, which host
    /// SDL does not set on a Mac.
    #[cfg(target_arch = "x86_64")]
    #[test]
    fn a_window_gets_the_x11_number_the_game_looks_for() {
        install_fakes();
        name_window(2);
        name_window(0);
        assert_eq!(
            *NUMBERS.lock().expect("numbers"),
            [(102, "SDL.window.x11.window".to_owned(), 3)]
        );
    }

    #[test]
    fn a_borderless_window_as_large_as_its_display_takes_the_screen() {
        let display = (2560, 1440);
        assert!(covers_display(0x1000_0610, display, display));
        // So does a window SDL holds full screen, whatever its size and border.
        assert!(covers_display(0x1000_0611, display, display));
        assert!(covers_display(0x1000_0401, (1920, 1080), display));
        // A window with a border, a smaller one, a hidden one, and a minimized
        // one do not.
        assert!(!covers_display(0x1000_0600, display, display));
        assert!(!covers_display(0x1000_0610, (1920, 1080), display));
        assert!(!covers_display(0x1000_0618, display, display));
        assert!(!covers_display(0x1000_0650, display, display));
        assert!(!covers_display(0x1000_0610, (0, 0), (0, 0)));
    }

    #[cfg(target_arch = "x86_64")]
    static PUSHED: Mutex<Vec<u32>> = Mutex::new(Vec::new());

    #[cfg(target_arch = "x86_64")]
    extern "C" fn push_event(event: *mut [u32; 32]) -> bool {
        // SAFETY: the bridge passes an event.
        PUSHED.lock().expect("pushed").push(unsafe { (*event)[0] });
        true
    }

    /// SIGTERM asks the game to quit with SDL's quit event, once, and puts the
    /// default action back for a second signal.
    #[cfg(target_arch = "x86_64")]
    #[test]
    fn a_signal_asks_the_game_to_quit_once() {
        install_fakes();
        quit_if_asked();
        assert!(PUSHED.lock().expect("pushed").is_empty());
        ask_to_quit(libc::SIGUSR1);
        quit_if_asked();
        quit_if_asked();
        assert_eq!(*PUSHED.lock().expect("pushed"), [EVENT_QUIT]);
    }

    #[test]
    fn the_peak_is_the_loudest_sample() {
        let samples = [0.0f32, -0.5, 0.25];
        // SAFETY: three samples, twelve bytes.
        let loudest = unsafe { peak(samples.as_ptr() as u64, 12) };
        assert!((loudest - 0.5).abs() < f32::EPSILON);
    }

    #[test]
    fn every_wrapped_function_has_a_wrapper() {
        for name in [
            "SDL_SetLogOutputFunction",
            "SDL_AddEventWatch",
            "SDL_RemoveEventWatch",
            "SDL_SetClipboardData",
            "SDL_OpenAudioDeviceStream",
            "SDL_DestroyAudioStream",
            "SDL_IOFromFile",
            "SDL_Vulkan_LoadLibrary",
            "SDL_MinimizeWindow",
            "SDL_PeepEvents",
            "SDL_SetWindowSize",
            "SDL_SetWindowPosition",
            "SDL_SetWindowBordered",
            "SDL_SetWindowFullscreen",
            "SDL_SetWindowFullscreenMode",
            "SDL_SyncWindow",
            "SDL_RestoreWindow",
            "SDL_DestroyWindow",
            "SDL_CreateWindowWithProperties",
        ] {
            assert!(wrapper(name.as_bytes()).is_some(), "{name}");
        }
        assert!(wrapper(b"SDL_Init").is_none());
    }
}
