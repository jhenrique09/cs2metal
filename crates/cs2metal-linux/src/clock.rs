//! The guest's clocks, read without macOS thread data.
//!
//! Everything here is *neutral* in the bridge's sense (docs/ARCHITECTURE.md): it
//! may run in the guest domain, with GS at the guest's control block, because it
//! reads no thread-specific data, takes no lock, allocates nothing, and cannot
//! panic. That is what lets the guest read the time through the vDSO
//! ([`crate::vdso`]) and through the system call entry's fast path without the
//! two GS switches a host call costs.
//!
//! One function answers every reader of a clock: the vDSO, the `clock_gettime`,
//! `gettimeofday`, and `time` system calls, and the absolute timeouts of `futex`
//! and `clock_nanosleep`. A deadline the guest computed from one of them is then
//! compared with the same clock.
//!
//! - The monotonic clocks are `mach_absolute_time`, which does not advance while
//!   the Mac sleeps, as Linux's `CLOCK_MONOTONIC` does not. `CLOCK_BOOTTIME` is
//!   `mach_continuous_time`, which does.
//! - The wall clock is `gettimeofday`, whose usual path reads the page the kernel
//!   shares with every process.
//! - The processor-time clocks need the thread's own state and are not answered
//!   here: [`now_ns`] returns `None` and the caller asks libSystem, in the host
//!   domain.
//!
//! A test runs these with GS at a block of poison words
//! (`cs2metal_loader::probe_neutral`), which is how "reads no thread data" is
//! known rather than assumed.

use std::sync::atomic::{AtomicU32, Ordering};

/// Linux clock ids.
pub(crate) const REALTIME: u64 = 0;
pub(crate) const MONOTONIC: u64 = 1;
pub(crate) const PROCESS_CPUTIME: u64 = 2;
pub(crate) const THREAD_CPUTIME: u64 = 3;
const MONOTONIC_RAW: u64 = 4;
const REALTIME_COARSE: u64 = 5;
const MONOTONIC_COARSE: u64 = 6;
const BOOTTIME: u64 = 7;

pub(crate) const NANOS: u64 = 1_000_000_000;

/// `mach_timebase_info_data_t`.
#[repr(C)]
struct Timebase {
    numer: u32,
    denom: u32,
}

unsafe extern "C" {
    fn mach_absolute_time() -> u64;
    fn mach_continuous_time() -> u64;
    fn mach_timebase_info(info: *mut Timebase) -> libc::c_int;
}

/// The ratio of nanoseconds to Mach time units; zero until [`init`].
static NUMER: AtomicU32 = AtomicU32::new(0);
static DENOM: AtomicU32 = AtomicU32::new(0);

/// Reads the Mach time base and calls each clock once, so that nothing is left to
/// look up the first time neutral code reads one. Call it in the host domain,
/// before the guest starts.
pub(crate) fn init() {
    let mut info = Timebase { numer: 0, denom: 0 };
    // SAFETY: mach_timebase_info fills the struct it is given.
    let status = unsafe { mach_timebase_info(&mut info) };
    let (numer, denom) = if status == 0 && info.numer != 0 && info.denom != 0 {
        (info.numer, info.denom)
    } else {
        (1, 1)
    };
    NUMER.store(numer, Ordering::Relaxed);
    DENOM.store(denom, Ordering::Relaxed);
    for clock in [REALTIME, MONOTONIC, BOOTTIME] {
        let _ = now_ns(clock);
    }
}

/// Mach time units as nanoseconds. Under Rosetta 2 they are nanoseconds already.
fn scale(ticks: u64) -> u64 {
    let (numer, denom) = (NUMER.load(Ordering::Relaxed), DENOM.load(Ordering::Relaxed));
    if numer == denom {
        // Also before `init`, when both are zero.
        return ticks;
    }
    (u128::from(ticks) * u128::from(numer) / u128::from(denom.max(1))) as u64
}

/// The Linux clock `clock` in nanoseconds, or `None` for a clock this module does
/// not answer: the processor-time clocks, and ids Linux does not have. Neutral.
pub(crate) fn now_ns(clock: u64) -> Option<u64> {
    match clock {
        MONOTONIC | MONOTONIC_RAW | MONOTONIC_COARSE => {
            // SAFETY: mach_absolute_time has no preconditions.
            Some(scale(unsafe { mach_absolute_time() }))
        }
        BOOTTIME => {
            // SAFETY: mach_continuous_time has no preconditions.
            Some(scale(unsafe { mach_continuous_time() }))
        }
        REALTIME | REALTIME_COARSE => {
            let mut now = libc::timeval {
                tv_sec: 0,
                tv_usec: 0,
            };
            // SAFETY: gettimeofday fills the struct it is given; no time zone asked.
            if unsafe { libc::gettimeofday(&mut now, std::ptr::null_mut()) } != 0 {
                return None;
            }
            Some(
                (now.tv_sec.max(0) as u64)
                    .wrapping_mul(NANOS)
                    .wrapping_add(now.tv_usec.max(0) as u64 * 1000),
            )
        }
        _ => None,
    }
}

/// Writes `ns` as a Linux `struct timespec` at `out`.
///
/// # Safety
///
/// `out` must point at 16 writable bytes.
pub(crate) unsafe fn write_timespec(out: u64, ns: u64) {
    // SAFETY: the caller's contract; two 64-bit words on both systems.
    unsafe {
        std::ptr::write_unaligned(
            out as *mut [i64; 2],
            [(ns / NANOS) as i64, (ns % NANOS) as i64],
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_monotonic_clocks_advance_and_the_wall_clock_is_todays() {
        init();
        let first = now_ns(MONOTONIC).expect("monotonic");
        std::thread::sleep(std::time::Duration::from_millis(20));
        let second = now_ns(MONOTONIC).expect("monotonic");
        let elapsed = second - first;
        assert!(
            (15_000_000..2_000_000_000).contains(&elapsed),
            "20 ms read as {elapsed} ns"
        );
        assert!(now_ns(MONOTONIC_RAW).expect("raw") >= second);
        assert!(now_ns(BOOTTIME).is_some());

        let wall = now_ns(REALTIME).expect("realtime") / NANOS;
        let system = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_secs();
        assert!(
            (system - 2..=system + 2).contains(&wall),
            "{wall} vs {system}"
        );
        assert_eq!(
            now_ns(REALTIME_COARSE).map(|ns| ns / NANOS / 10),
            Some(wall / 10)
        );
    }

    #[test]
    fn the_processor_time_clocks_are_left_to_the_host() {
        assert_eq!(now_ns(PROCESS_CPUTIME), None);
        assert_eq!(now_ns(THREAD_CPUTIME), None);
        assert_eq!(now_ns(11), None);
        // A dynamic clock id, as pthread_getcpuclockid makes: negative as an int.
        assert_eq!(now_ns(-6i32 as u32 as u64), None);
    }

    #[test]
    fn a_timespec_is_seconds_and_nanoseconds() {
        let mut out = [0i64; 2];
        // SAFETY: out is 16 writable bytes.
        unsafe { write_timespec(std::ptr::from_mut(&mut out) as u64, 3 * NANOS + 7) };
        assert_eq!(out, [3, 7]);
    }

    /// The clocks read no macOS thread data: they run with GS at a block of
    /// poison words, which following or changing would show.
    #[cfg(target_arch = "x86_64")]
    #[test]
    fn the_clocks_are_neutral() {
        extern "C" fn read(clock: u64) -> u64 {
            let mut sum = 0u64;
            for _ in 0..1000 {
                sum = sum.wrapping_add(now_ns(clock).unwrap_or(0) & 1);
            }
            let _ = sum;
            now_ns(clock).unwrap_or(0)
        }
        init();
        for clock in [REALTIME, MONOTONIC, BOOTTIME] {
            let value = cs2metal_loader::probe_neutral(read, clock).expect("neutral");
            assert_ne!(value, 0, "clock {clock}");
        }
    }
}
