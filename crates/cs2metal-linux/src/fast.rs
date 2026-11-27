//! The system calls answered in the guest's domain, without a GS switch.
//!
//! A system call costs two GS switches around the host code that answers it,
//! half a microsecond for the pair. The calls here need nothing of the host's
//! thread data, so the system call entry asks this handler first, before it
//! switches: a wake or a wait on a futex goes straight to the kernel, one trap
//! where there were three, and a read of the clock by a caller that does not go
//! through the vDSO makes none at all.
//!
//! Everything here must be neutral, as [`crate::clock`] says: no thread-local
//! storage, no `errno`, no lock, no allocation, no panic, nothing written to
//! standard error. A test runs each call with GS at a block of poison words.
//! Whatever a call would need beyond that, it declines, and the host handler
//! answers it as before: a futex operation the host does not know, which is
//! named on standard error, and every call while a trace is on, since a trace
//! writes and takes locks.

use std::sync::atomic::{AtomicBool, Ordering};

use cs2metal_loader::SyscallFrame;

static ON: AtomicBool = AtomicBool::new(false);

/// Linux system call numbers.
const SCHED_YIELD: u64 = 24;
const GETTIMEOFDAY: u64 = 96;
const TIME: u64 = 201;
const FUTEX: u64 = 202;
const CLOCK_GETTIME: u64 = 228;

/// Whether the environment lets calls be answered here: not with
/// `CS2METAL_NO_FAST_SYSCALLS`, and not while the system call trace or the futex
/// trace is on, which must see every call in the host domain.
fn allowed(set: impl Fn(&str) -> bool) -> bool {
    ![
        "CS2METAL_NO_FAST_SYSCALLS",
        "CS2METAL_TRACE",
        "CS2METAL_FUTEX_TRACE",
    ]
    .iter()
    .any(|name| set(name))
}

/// Installs the handler, in the host domain, before the guest starts.
pub(crate) fn start() {
    crate::clock::init();
    ON.store(
        allowed(|name| std::env::var_os(name).is_some()),
        Ordering::Relaxed,
    );
    cs2metal_loader::set_fast_syscall_handler(handle);
}

/// Answers `f` if it is one of the calls above, writing the result where the
/// entry returns it; `false` leaves it to the host handler. Neutral.
pub(crate) extern "C" fn handle(f: &mut SyscallFrame) -> bool {
    if !ON.load(Ordering::Relaxed) {
        return false;
    }
    let value: i64 = match f.number {
        FUTEX => match crate::threads::futex_neutral(f.arg0, f.arg1, f.arg2, f.arg3) {
            Some(value) => value,
            None => return false,
        },
        CLOCK_GETTIME => {
            let value = crate::vdso::clock_gettime(f.arg0 as i32, f.arg1);
            // The clocks the vDSO leaves to the system call need the host.
            if value != 0 {
                return false;
            }
            value
        }
        GETTIMEOFDAY => crate::vdso::gettimeofday(f.arg0, f.arg1),
        TIME => crate::vdso::time(f.arg0),
        // SAFETY: sched_yield has no preconditions.
        SCHED_YIELD => i64::from(unsafe { libc::sched_yield() }),
        _ => return false,
    };
    f.number = value as u64;
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::translate::{EAGAIN, ETIMEDOUT};

    fn frame(number: u64, args: [u64; 4]) -> SyscallFrame {
        SyscallFrame {
            number,
            arg0: args[0],
            arg1: args[1],
            arg2: args[2],
            arg3: args[3],
            arg4: 0,
            arg5: 0,
            saved_rbx: 0,
            saved_rbp: 0,
            return_address: 0,
        }
    }

    /// The result of a call the handler answers, or `None` for one it declines.
    fn call(number: u64, args: [u64; 4]) -> Option<i64> {
        let mut f = frame(number, args);
        handle(&mut f).then_some(f.number as i64)
    }

    #[test]
    fn a_trace_keeps_every_call_in_the_host_domain() {
        assert!(allowed(|_| false));
        for name in [
            "CS2METAL_NO_FAST_SYSCALLS",
            "CS2METAL_TRACE",
            "CS2METAL_FUTEX_TRACE",
        ] {
            assert!(!allowed(|set| set == name), "{name}");
        }
        assert!(allowed(|set| set == "CS2METAL_VULKAN_TRACE"));
    }

    const WAIT: u64 = 0x80; // FUTEX_WAIT | FUTEX_PRIVATE_FLAG
    const WAKE: u64 = 0x81;

    /// Every call the handler answers, made as the guest makes it. Returns the
    /// number of the first check that fails, or zero.
    extern "C" fn every_call(_: u64) -> u64 {
        let word = std::sync::atomic::AtomicU32::new(7);
        let addr = std::ptr::from_ref(&word) as u64;
        let timeout = [0i64, 2_000_000];
        let mut ts = [0i64; 2];
        let mut tv = [0i64; 2];
        let checks = [
            // Nobody waits: the host's ENOENT, which Linux reports as none woken.
            call(FUTEX, [addr, WAKE, 1, 0]) == Some(0),
            call(FUTEX, [addr, WAKE, u64::from(u32::MAX), 0]) == Some(0),
            // The word differs from the value waited against.
            call(FUTEX, [addr, WAIT, 8, 0]) == Some(EAGAIN),
            // Two milliseconds, and nobody wakes.
            call(FUTEX, [addr, WAIT, 7, std::ptr::from_ref(&timeout) as u64]) == Some(ETIMEDOUT),
            // FUTEX_REQUEUE is not answered at all.
            call(FUTEX, [addr, 3, 1, 0]).is_none(),
            call(CLOCK_GETTIME, [1, std::ptr::from_mut(&mut ts) as u64, 0, 0]) == Some(0),
            ts[0] > 0,
            // The thread's processor time is the host's to answer.
            call(CLOCK_GETTIME, [3, std::ptr::from_mut(&mut ts) as u64, 0, 0]).is_none(),
            call(GETTIMEOFDAY, [std::ptr::from_mut(&mut tv) as u64, 0, 0, 0]) == Some(0),
            call(TIME, [0; 4]).is_some_and(|now| (tv[0]..=tv[0] + 2).contains(&now)),
            call(SCHED_YIELD, [0; 4]) == Some(0),
            // getpid is no business of this handler.
            call(39, [0; 4]).is_none(),
        ];
        checks.iter().position(|ok| !ok).map_or(0, |i| i as u64 + 1)
    }

    /// The handler answers its calls, and declines the rest, in both domains;
    /// in the guest's, with GS at a block of poison words, it touches no thread
    /// data on the way: not on a wake that finds nobody, nor on a wait that
    /// times out, which are the host's error paths.
    #[test]
    fn the_calls_are_answered_and_neutral() {
        crate::clock::init();
        ON.store(true, Ordering::Relaxed);
        assert_eq!(every_call(0), 0, "in the host domain");
        #[cfg(target_arch = "x86_64")]
        assert_eq!(
            cs2metal_loader::probe_neutral(every_call, 0).expect("neutral"),
            0,
            "in the guest domain"
        );
    }

    /// A wait in the guest's domain ends when another thread wakes its word.
    #[cfg(target_arch = "x86_64")]
    #[test]
    fn a_neutral_wait_is_woken() {
        static WORD: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        extern "C" fn wait(_: u64) -> u64 {
            let addr = std::ptr::from_ref(&WORD) as u64;
            while WORD.load(Ordering::Relaxed) == 0 {
                if call(FUTEX, [addr, WAIT, 0, 0]).is_none() {
                    return 1;
                }
            }
            0
        }
        ON.store(true, Ordering::Relaxed);
        let waker = std::thread::spawn(|| {
            std::thread::sleep(std::time::Duration::from_millis(30));
            WORD.store(1, Ordering::Relaxed);
            let addr = std::ptr::from_ref(&WORD) as u64;
            // Until the waiter is gone: it may not be asleep yet.
            for _ in 0..200 {
                call(FUTEX, [addr, WAKE, 1, 0]);
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
        });
        assert_eq!(cs2metal_loader::probe_neutral(wait, 0).expect("neutral"), 0);
        waker.join().expect("waker");
    }
}
