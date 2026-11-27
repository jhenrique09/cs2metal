//! Linux threads on host threads: `clone`, thread exit, `futex`, and thread ids.
//!
//! A guest thread runs on its own host pthread. `clone` makes the host thread,
//! which builds its own host block (the bridge needs that thread's TSD base),
//! attaches the guest's new control block to it, and resumes the guest where the
//! kernel would: after the system call, with `rax` 0 and the new stack. When the
//! guest thread calls `exit`, the host thread returns to its own stack before it
//! clears the thread id and wakes whoever joins, because glibc may free the guest
//! stack as soon as the id reads zero.

use std::cell::Cell;
use std::sync::atomic::{AtomicU32, Ordering};

use cs2metal_loader::{HostBlock, SyscallFrame, attach, leave_guest_thread, run_guest_thread};

use crate::syscall::BLOCK;
use crate::translate::{EAGAIN, EINVAL, ENOSYS, ETIMEDOUT, result};

const CLONE_VM: u64 = 0x100;
const CLONE_THREAD: u64 = 0x10000;
const CLONE_SETTLS: u64 = 0x80000;
const CLONE_PARENT_SETTID: u64 = 0x100000;
const CLONE_CHILD_CLEARTID: u64 = 0x200000;
const CLONE_CHILD_SETTID: u64 = 0x1000000;

/// Thread ids for the guest: the main thread's is the process id.
static NEXT_TID: AtomicU32 = AtomicU32::new(0);

thread_local! {
    static TID: Cell<u32> = const { Cell::new(0) };
    /// `set_tid_address` and `CLONE_CHILD_CLEARTID`: cleared and woken at exit.
    static CLEAR_TID: Cell<u64> = const { Cell::new(0) };
    /// Whether this host thread runs a guest thread made by clone.
    static CLONED: Cell<bool> = const { Cell::new(false) };
}

pub(crate) fn gettid() -> i64 {
    TID.with(|t| {
        if t.get() == 0 {
            // SAFETY: getpid has no preconditions.
            t.set(unsafe { libc::getpid() } as u32);
        }
        i64::from(t.get())
    })
}

fn new_tid() -> u32 {
    // SAFETY: getpid has no preconditions.
    let base = unsafe { libc::getpid() } as u32;
    base + 1 + NEXT_TID.fetch_add(1, Ordering::Relaxed)
}

pub(crate) fn set_tid_address(ptr: u64) -> i64 {
    CLEAR_TID.with(|c| c.set(ptr));
    gettid()
}

unsafe extern "C" {
    fn os_sync_wait_on_address(addr: *mut libc::c_void, value: u64, size: usize, flags: u32)
    -> i32;
    fn os_sync_wait_on_address_with_timeout(
        addr: *mut libc::c_void,
        value: u64,
        size: usize,
        flags: u32,
        clock: u32,
        timeout_ns: u64,
    ) -> i32;
    fn os_sync_wake_by_address_any(addr: *mut libc::c_void, size: usize, flags: u32) -> i32;
    fn os_sync_wake_by_address_all(addr: *mut libc::c_void, size: usize, flags: u32) -> i32;
}

/// `OS_CLOCK_MACH_ABSOLUTE_TIME`.
const OS_CLOCK_MACH_ABSOLUTE_TIME: u32 = 32;

fn wake(addr: u64, count: u64) -> i64 {
    if addr == 0 {
        return EINVAL;
    }
    let p = addr as *mut libc::c_void;
    // SAFETY: the guest names a 32-bit futex word in its own memory; waking has no
    // effect on it.
    let status = unsafe {
        if count > 1 {
            os_sync_wake_by_address_all(p, 4, 0)
        } else {
            os_sync_wake_by_address_any(p, 4, 0)
        }
    };
    // Linux returns the number woken; glibc only tests for errors. ENOENT from
    // the host means nobody was waiting.
    if status == 0 { 1 } else { 0 }
}

/// Nanoseconds in a Linux `struct timespec`.
///
/// # Safety
///
/// `ts` must point at a `struct timespec`.
unsafe fn nanos(ts: u64) -> u64 {
    // SAFETY: the caller's contract; the layout is two 64-bit words on both.
    let [s, ns] = unsafe { std::ptr::read_unaligned(ts as *const [i64; 2]) };
    (s.max(0) as u64)
        .saturating_mul(1_000_000_000)
        .saturating_add(ns.max(0) as u64)
}

fn now_ns(clock: libc::clockid_t) -> u64 {
    let mut ts = libc::timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    // SAFETY: clock_gettime writes into the struct.
    unsafe { libc::clock_gettime(clock, &mut ts) };
    (ts.tv_sec.max(0) as u64)
        .saturating_mul(1_000_000_000)
        .saturating_add(ts.tv_nsec.max(0) as u64)
}

/// `futex(addr, op, val, timeout, addr2, val3)`.
pub(crate) fn futex(addr: u64, op: u64, val: u64, timeout: u64, _addr2: u64, _val3: u64) -> i64 {
    const WAIT: u64 = 0;
    const WAKE: u64 = 1;
    const WAIT_BITSET: u64 = 9;
    const WAKE_BITSET: u64 = 10;
    const CLOCK_REALTIME: u64 = 256;
    let cmd = op & 0x7f;
    match cmd {
        WAKE | WAKE_BITSET => wake(addr, val),
        WAIT | WAIT_BITSET => {
            if addr == 0 {
                return EINVAL;
            }
            // SAFETY: the guest names a 32-bit futex word in its own memory.
            let current = unsafe { std::ptr::read_volatile(addr as *const u32) };
            if current != val as u32 {
                return EAGAIN;
            }
            // WAIT takes a relative timeout; WAIT_BITSET an absolute one, on
            // CLOCK_MONOTONIC unless FUTEX_CLOCK_REALTIME.
            let relative = (timeout != 0).then(|| {
                // SAFETY: the guest passes a struct timespec.
                let t = unsafe { nanos(timeout) };
                if cmd == WAIT {
                    t
                } else {
                    let base = if op & CLOCK_REALTIME != 0 {
                        now_ns(libc::CLOCK_REALTIME)
                    } else {
                        now_ns(libc::CLOCK_MONOTONIC)
                    };
                    t.saturating_sub(base)
                }
            });
            let p = addr as *mut libc::c_void;
            // SAFETY: as above; the host compares and sleeps atomically.
            let status = unsafe {
                match relative {
                    None => os_sync_wait_on_address(p, val & 0xffff_ffff, 4, 0),
                    Some(0) => return ETIMEDOUT,
                    Some(ns) => os_sync_wait_on_address_with_timeout(
                        p,
                        val & 0xffff_ffff,
                        4,
                        0,
                        OS_CLOCK_MACH_ABSOLUTE_TIME,
                        ns,
                    ),
                }
            };
            if status >= 0 {
                0
            } else {
                let error = std::io::Error::last_os_error().raw_os_error().unwrap_or(0);
                if error == libc::ETIMEDOUT {
                    ETIMEDOUT
                } else if error == libc::EINTR || error == libc::EFAULT {
                    result(-1)
                } else {
                    0
                }
            }
        }
        _ => {
            unsupported_futex(cmd);
            ENOSYS
        }
    }
}

// --- The futex trace ---------------------------------------------------------------

/// A guest thread inside a futex wait.
struct Waiter {
    addr: u64,
    /// The value the wait was made against.
    value: u32,
    timed: bool,
    since: std::time::Instant,
    /// Where the guest made the call, and the caller of glibc's `syscall()` if
    /// the call went through it.
    site: u64,
    via: u64,
}

/// The wakes of one futex word since the last report.
#[derive(Default)]
struct Wakes {
    calls: u64,
    /// Calls that found a waiter.
    woke: u64,
    tid: u32,
    site: u64,
    via: u64,
}

#[derive(Default)]
struct Trace {
    waiters: std::collections::BTreeMap<u32, Waiter>,
    wakes: std::collections::BTreeMap<u64, Wakes>,
}

static TRACE: std::sync::Mutex<Option<Trace>> = std::sync::Mutex::new(None);

/// How long a wait lasts before a report names it.
const PARKED: std::time::Duration = std::time::Duration::from_secs(2);

/// The seconds between reports: `CS2METAL_FUTEX_TRACE`, 10 if it is not a number.
/// `None` without the variable: no trace.
fn trace_period() -> Option<u64> {
    static PERIOD: std::sync::OnceLock<Option<u64>> = std::sync::OnceLock::new();
    *PERIOD.get_or_init(|| {
        let value = std::env::var("CS2METAL_FUTEX_TRACE").ok()?;
        Some(value.parse().ok().filter(|&s| s > 0).unwrap_or(10))
    })
}

/// The report's lines: every wait older than [`PARKED`], oldest first, with what
/// the word holds now (`holds`, read by the caller), then the most woken words.
/// A wait whose word no longer holds its value, on a word that is being woken,
/// is a wake that did not arrive.
fn report(trace: &Trace, now: std::time::Instant, holds: impl Fn(u64) -> u32) -> Vec<String> {
    let mut parked: Vec<(&u32, &Waiter)> = trace
        .waiters
        .iter()
        .filter(|(_, w)| now.duration_since(w.since) >= PARKED)
        .collect();
    parked.sort_by_key(|(_, w)| w.since);
    let mut lines = vec![format!(
        "cs2metal: futex report: {} waiting, {} of them parked, {} words woken",
        trace.waiters.len(),
        parked.len(),
        trace.wakes.len()
    )];
    for (tid, w) in parked {
        lines.push(format!(
            "cs2metal: futex thread {tid} parked {}s on {:#x} for {}, holds {}{} from {:#x} via {:#x}",
            now.duration_since(w.since).as_secs(),
            w.addr,
            w.value,
            holds(w.addr),
            if w.timed { ", timed" } else { "" },
            w.site,
            w.via
        ));
    }
    let mut woken: Vec<(&u64, &Wakes)> = trace.wakes.iter().collect();
    woken.sort_by_key(|(_, k)| std::cmp::Reverse(k.calls));
    for (addr, k) in woken.into_iter().take(16) {
        let waiting = trace.waiters.values().filter(|w| w.addr == *addr).count();
        lines.push(format!(
            "cs2metal: futex word {addr:#x} woken {} times, {} found a waiter, {waiting} waiting now, \
             last by thread {} from {:#x} via {:#x}",
            k.calls, k.woke, k.tid, k.site, k.via
        ));
    }
    lines
}

/// Prints a report every `period` seconds, on a host thread.
fn start_reports(period: u64) {
    static STARTED: std::sync::Once = std::sync::Once::new();
    STARTED.call_once(|| {
        std::thread::spawn(move || {
            loop {
                std::thread::sleep(std::time::Duration::from_secs(period));
                let Ok(mut trace) = TRACE.lock() else {
                    return;
                };
                let Some(trace) = trace.as_mut() else {
                    continue;
                };
                // SAFETY: a thread is waiting on each word read, so it is mapped.
                let holds = |addr: u64| unsafe { std::ptr::read_volatile(addr as *const u32) };
                for line in report(trace, std::time::Instant::now(), holds) {
                    eprintln!("{line}");
                }
                trace.wakes.clear();
            }
        });
    });
}

/// `futex` as the guest's system call `f` makes it. With `CS2METAL_FUTEX_TRACE`
/// set, the waits in progress and the wakes are recorded for the reports.
pub(crate) fn futex_call(f: &SyscallFrame) -> i64 {
    let call = || futex(f.arg0, f.arg1, f.arg2, f.arg3, f.arg4, f.arg5);
    let Some(period) = trace_period() else {
        return call();
    };
    start_reports(period);
    let site = f.return_address;
    // glibc's syscall() makes room for the red zone and calls through the slot,
    // so its own return address is 128 bytes above the entry's.
    // SAFETY: the guest's stack above the entry's frame.
    let via =
        unsafe { std::ptr::read_unaligned(std::ptr::from_ref(&f.return_address).byte_add(136)) };
    let tid = gettid() as u32;
    let cmd = f.arg1 & 0x7f;
    let with = |change: &mut dyn FnMut(&mut Trace)| {
        if let Ok(mut trace) = TRACE.lock() {
            change(trace.get_or_insert_with(Trace::default));
        }
    };
    match cmd {
        0 | 9 => {
            with(&mut |t| {
                t.waiters.insert(
                    tid,
                    Waiter {
                        addr: f.arg0,
                        value: f.arg2 as u32,
                        timed: f.arg3 != 0,
                        since: std::time::Instant::now(),
                        site,
                        via,
                    },
                );
            });
            let result = call();
            with(&mut |t| {
                t.waiters.remove(&tid);
            });
            result
        }
        1 | 10 => {
            let result = call();
            with(&mut |t| {
                let k = t.wakes.entry(f.arg0).or_default();
                k.calls += 1;
                k.woke += u64::from(result > 0);
                (k.tid, k.site, k.via) = (tid, site, via);
            });
            result
        }
        _ => call(),
    }
}

/// Names each futex operation the host does not answer, once.
fn unsupported_futex(cmd: u64) {
    static SEEN: std::sync::Mutex<u64> = std::sync::Mutex::new(0);
    if let Ok(mut seen) = SEEN.lock() {
        if cmd < 64 && *seen & (1 << cmd) == 0 {
            *seen |= 1 << cmd;
            eprintln!("cs2metal: unsupported futex operation {cmd}");
        }
    }
}

/// What a new host thread needs to resume the guest.
struct Start {
    tls: u64,
    rip: u64,
    rsp: u64,
    tid: u32,
    clear_tid: u64,
    set_tid: u64,
}

extern "C" fn thread_main(arg: *mut libc::c_void) -> *mut libc::c_void {
    // SAFETY: clone passes a leaked Box<Start>.
    let start = unsafe { Box::from_raw(arg.cast::<Start>()) };
    let block = match HostBlock::new(start.tls) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("cs2metal: a guest thread could not start: {e}");
            std::process::abort();
        }
    };
    // SAFETY: tls is the control block glibc made for the thread.
    unsafe { attach(start.tls, &block) };
    TID.with(|t| t.set(start.tid));
    CLEAR_TID.with(|c| c.set(start.clear_tid));
    CLONED.with(|c| c.set(true));
    if start.set_tid != 0 {
        // SAFETY: CLONE_CHILD_SETTID names a word in guest memory.
        unsafe { std::ptr::write_volatile(start.set_tid as *mut u32, start.tid) };
    }
    BLOCK.with(|b| *b.borrow_mut() = Some(block));
    // No RefCell borrow may live across the guest: leave_guest_thread jumps back
    // here and would leak it. The block stays in place inside the thread-local
    // until it is taken out below, so a raw pointer to it is stable meanwhile.
    let block = current_block();
    if let Some(block) = block {
        // SAFETY: the block belongs to this thread, is attached to tls, and lives
        // until it is taken out of BLOCK below; rip and rsp are where the kernel
        // would resume the new thread.
        unsafe { run_guest_thread(start.tls, start.rip, start.rsp, &*block) };
    }
    // Back on the host stack: the guest thread is gone. Clear its id and wake a
    // joiner, as the kernel does for CLONE_CHILD_CLEARTID.
    let clear = CLEAR_TID.with(Cell::get);
    if clear != 0 {
        // SAFETY: the word glibc gave for the thread id, in guest memory.
        unsafe { std::ptr::write_volatile(clear as *mut u32, 0) };
        wake(clear, u64::MAX);
    }
    BLOCK.with(|b| *b.borrow_mut() = None);
    std::ptr::null_mut()
}

/// `clone(flags, stack, parent_tid, child_tid, tls)` for glibc's threads.
pub(crate) fn clone(f: &SyscallFrame) -> i64 {
    let (flags, stack, ptid, ctid, tls) = (f.arg0, f.arg1, f.arg2, f.arg3, f.arg4);
    let thread = CLONE_VM | CLONE_THREAD | CLONE_SETTLS;
    if flags & thread != thread || stack == 0 {
        // A new process (fork) or a thread without its own TLS: not yet.
        return ENOSYS;
    }
    let tid = new_tid();
    if flags & CLONE_PARENT_SETTID != 0 && ptid != 0 {
        // SAFETY: the word glibc gave for the parent's copy of the id.
        unsafe { std::ptr::write_volatile(ptid as *mut u32, tid) };
    }
    let start = Box::new(Start {
        tls,
        rip: f.return_address,
        // The guest did `lea -128(%rsp), %rsp` before its call and does
        // `lea 128(%rsp), %rsp` after it; the new thread resumes as if it had
        // done the first on its new stack.
        rsp: stack - 128,
        tid,
        clear_tid: if flags & CLONE_CHILD_CLEARTID != 0 {
            ctid
        } else {
            0
        },
        set_tid: if flags & CLONE_CHILD_SETTID != 0 {
            ctid
        } else {
            0
        },
    });
    // SAFETY: attr is initialized before use and destroyed after.
    unsafe {
        let mut attr: libc::pthread_attr_t = std::mem::zeroed();
        libc::pthread_attr_init(&mut attr);
        libc::pthread_attr_setdetachstate(&mut attr, libc::PTHREAD_CREATE_DETACHED);
        libc::pthread_attr_setstacksize(&mut attr, 1 << 20);
        let mut thread: libc::pthread_t = std::mem::zeroed();
        let arg = Box::into_raw(start).cast();
        let status = libc::pthread_create(&mut thread, &attr, thread_main, arg);
        libc::pthread_attr_destroy(&mut attr);
        if status != 0 {
            drop(Box::from_raw(arg.cast::<Start>()));
            return EAGAIN;
        }
    }
    i64::from(tid)
}

/// `exit`: ends a guest thread made by clone; on the main thread, the process.
pub(crate) fn exit_thread(status: u64) -> i64 {
    if !CLONED.with(Cell::get) {
        let _ = std::io::Write::flush(&mut std::io::stdout());
        std::process::exit(status as i32);
    }
    match current_block() {
        // SAFETY: called from this guest thread's own system call, in the host
        // domain, on the host thread that run_guest_thread runs; the block lives in
        // the thread-local until thread_main takes it out after this returns there.
        Some(block) => unsafe { leave_guest_thread(&*block, status) },
        None => EINVAL,
    }
}

/// A pointer to this thread's host block, read without holding a borrow.
fn current_block() -> Option<*const HostBlock> {
    BLOCK.with(|b| b.borrow().as_ref().map(std::ptr::from_ref))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicBool;
    use std::time::{Duration, Instant};

    const WAIT: u64 = 0x80;
    const WAKE: u64 = 0x81;

    #[test]
    fn a_wait_ends_when_its_word_is_woken() {
        static WORD: AtomicU32 = AtomicU32::new(0);
        static DONE: AtomicBool = AtomicBool::new(false);
        let addr = std::ptr::from_ref(&WORD) as u64;
        let waiter = std::thread::spawn(move || {
            while WORD.load(Ordering::SeqCst) == 0 {
                futex(addr, WAIT, 0, 0, 0, 0);
            }
            DONE.store(true, Ordering::SeqCst);
        });
        std::thread::sleep(Duration::from_millis(50));
        WORD.store(1, Ordering::SeqCst);
        let deadline = Instant::now() + Duration::from_secs(10);
        while !DONE.load(Ordering::SeqCst) && Instant::now() < deadline {
            futex(addr, WAKE, i32::MAX as u64, 0, 0, 0);
            std::thread::sleep(Duration::from_millis(1));
        }
        assert!(DONE.load(Ordering::SeqCst), "the waiter was woken");
        waiter.join().expect("waiter");
    }

    #[test]
    fn a_wait_checks_the_value_and_the_timeout() {
        let word = AtomicU32::new(7);
        let addr = std::ptr::from_ref(&word) as u64;
        assert_eq!(futex(addr, WAIT, 8, 0, 0, 0), EAGAIN);
        let millisecond: [i64; 2] = [0, 1_000_000];
        let timeout = std::ptr::from_ref(&millisecond) as u64;
        assert_eq!(futex(addr, WAIT, 7, timeout, 0, 0), ETIMEDOUT);
        assert_eq!(futex(addr, WAKE, 1, 0, 0, 0), 0, "nobody was waiting");
    }

    #[test]
    fn a_report_names_parked_waits_and_woken_words() {
        let now = Instant::now() + Duration::from_secs(60);
        let wait = |addr, age| Waiter {
            addr,
            value: 0,
            timed: false,
            since: now - Duration::from_secs(age),
            site: 0x1000,
            via: 0x2000,
        };
        let mut trace = Trace::default();
        trace.waiters.insert(11, wait(0x500, 30));
        trace.waiters.insert(12, wait(0x600, 0));
        trace.wakes.insert(
            0x500,
            Wakes {
                calls: 9,
                woke: 0,
                tid: 10,
                site: 0x3000,
                via: 0x4000,
            },
        );
        let lines = report(&trace, now, |_| 1);
        assert_eq!(
            lines,
            [
                "cs2metal: futex report: 2 waiting, 1 of them parked, 1 words woken",
                "cs2metal: futex thread 11 parked 30s on 0x500 for 0, holds 1 from 0x1000 via 0x2000",
                "cs2metal: futex word 0x500 woken 9 times, 0 found a waiter, 1 waiting now, \
                 last by thread 10 from 0x3000 via 0x4000",
            ]
        );
    }
}
