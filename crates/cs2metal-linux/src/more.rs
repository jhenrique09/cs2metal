//! System calls the engine needs beyond the fixtures' set: files and directories,
//! pipes and events, time and sleep, memory, process information, and signals
//! sent to itself. `None` means the number is not handled here.

use std::ffi::{CStr, c_char};

use cs2metal_loader::SyscallFrame;

use crate::fds;
use crate::paths::Use;
use crate::syscall::host_path;
use crate::translate::{EFAULT, EINVAL, ENOMEM, dirfd, result};

fn write_fds(out: u64, fds: [libc::c_int; 2]) -> i64 {
    if out == 0 {
        return EFAULT;
    }
    // SAFETY: the guest passes an int[2].
    unsafe { std::ptr::write_unaligned(out as *mut [libc::c_int; 2], fds) };
    0
}

fn pipe2(out: u64, flags: u64) -> i64 {
    let mut fds = [0 as libc::c_int; 2];
    // SAFETY: pipe writes two descriptors into the array.
    if unsafe { libc::pipe(fds.as_mut_ptr()) } != 0 {
        return result(-1);
    }
    for fd in fds {
        // SAFETY: fcntl on the descriptors just made.
        unsafe {
            if flags & 0o2000000 != 0 {
                libc::fcntl(fd, libc::F_SETFD, libc::FD_CLOEXEC);
            }
            if flags & 0o4000 != 0 {
                let f = libc::fcntl(fd, libc::F_GETFL);
                libc::fcntl(fd, libc::F_SETFL, f | libc::O_NONBLOCK);
            }
        }
    }
    write_fds(out, fds)
}

/// Linux `struct sysinfo`: 112 bytes.
fn sysinfo(out: u64) -> i64 {
    if out == 0 {
        return EFAULT;
    }
    let mut memory: u64 = 0;
    let mut size = std::mem::size_of::<u64>();
    // SAFETY: sysctlbyname writes a u64 for hw.memsize.
    unsafe {
        libc::sysctlbyname(
            c"hw.memsize".as_ptr(),
            std::ptr::from_mut(&mut memory).cast(),
            &mut size,
            std::ptr::null_mut(),
            0,
        )
    };
    let mut ts = libc::timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    // SAFETY: clock_gettime writes into the struct.
    unsafe { libc::clock_gettime(libc::CLOCK_MONOTONIC, &mut ts) };
    let mut words = [0u64; 14];
    words[0] = ts.tv_sec as u64; // uptime
    words[4] = memory; // totalram
    words[5] = memory / 2; // freeram: macOS has no single figure; half is a fair guess
    words[10] = 1; // procs (u16) in the low bytes; mem_unit below
    // SAFETY: the guest passes a struct sysinfo of 112 bytes.
    unsafe {
        std::ptr::write_bytes(out as *mut u8, 0, 112);
        std::ptr::copy_nonoverlapping(words.as_ptr().cast::<u8>(), out as *mut u8, 80);
        std::ptr::write_unaligned((out + 104) as *mut u32, 1); // mem_unit
    }
    0
}

fn sleep(request: u64) -> i64 {
    if request == 0 {
        return EFAULT;
    }
    // SAFETY: struct timespec has the same layout on both systems on x86-64.
    result(i64::from(unsafe {
        libc::nanosleep(request as *const libc::timespec, std::ptr::null_mut())
    }))
}

/// `clock_nanosleep(clock, flags, request, remain)`: TIMER_ABSTIME is 1.
fn clock_nanosleep(clock: u64, flags: u64, request: u64) -> i64 {
    if flags & 1 == 0 {
        return sleep(request);
    }
    // The clock the guest computed its deadline from.
    let clock = clock & 0xffff_ffff;
    let current = match crate::clock::now_ns(clock) {
        Some(ns) => i128::from(ns),
        None => {
            let Some(clock) = crate::translate::cpu_clock(clock) else {
                return EINVAL;
            };
            let mut now = libc::timespec {
                tv_sec: 0,
                tv_nsec: 0,
            };
            // SAFETY: clock_gettime writes into the struct.
            unsafe { libc::clock_gettime(clock, &mut now) };
            now.tv_sec as i128 * 1_000_000_000 + now.tv_nsec as i128
        }
    };
    // SAFETY: the guest passes a struct timespec.
    let [s, ns] = unsafe { std::ptr::read_unaligned(request as *const [i64; 2]) };
    let target = s as i128 * 1_000_000_000 + ns as i128;
    let wait = (target - current).max(0);
    let ts = libc::timespec {
        tv_sec: (wait / 1_000_000_000) as libc::time_t,
        tv_nsec: (wait % 1_000_000_000) as libc::c_long,
    };
    sleep(std::ptr::from_ref(&ts) as u64)
}

/// `mremap(old, old_size, new_size, flags)`: MREMAP_MAYMOVE is 1. macOS cannot
/// move a mapping, so a grown one is copied to a new anonymous mapping, which is
/// what glibc's realloc uses mremap for.
fn mremap(old: u64, old_size: u64, new_size: u64, flags: u64) -> i64 {
    if new_size <= old_size {
        if new_size < old_size {
            // SAFETY: the tail of a mapping the guest owns.
            unsafe { libc::munmap((old + new_size) as *mut _, (old_size - new_size) as usize) };
        }
        return old as i64;
    }
    if flags & 1 == 0 {
        return ENOMEM;
    }
    // SAFETY: a fresh anonymous mapping, then a copy of the guest's old one.
    unsafe {
        let new = libc::mmap(
            std::ptr::null_mut(),
            new_size as usize,
            libc::PROT_READ | libc::PROT_WRITE,
            libc::MAP_PRIVATE | libc::MAP_ANON,
            -1,
            0,
        );
        if new == libc::MAP_FAILED {
            return ENOMEM;
        }
        std::ptr::copy_nonoverlapping(old as *const u8, new.cast::<u8>(), old_size as usize);
        libc::munmap(old as *mut _, old_size as usize);
        new as i64
    }
}

/// Linux `struct statfs`: 120 bytes.
fn statfs(st: &libc::statfs, out: u64) -> i64 {
    let words: [u64; 7] = [
        0x6969, // f_type: report NFS-like, which nothing special-cases
        u64::from(st.f_bsize),
        st.f_blocks,
        st.f_bfree,
        st.f_bavail,
        st.f_files,
        st.f_ffree,
    ];
    // SAFETY: the guest passes a struct statfs of 120 bytes.
    unsafe {
        std::ptr::write_bytes(out as *mut u8, 0, 120);
        std::ptr::copy_nonoverlapping(words.as_ptr().cast::<u8>(), out as *mut u8, 56);
        std::ptr::write_unaligned((out + 64) as *mut u64, 255); // f_namelen
        std::ptr::write_unaligned((out + 72) as *mut u64, u64::from(st.f_bsize)); // f_frsize
    }
    0
}

/// `gettimeofday(tv, tz)`. Linux's `struct timeval` is two 64-bit words. The
/// host's holds its microseconds in 32 bits, so the host's own call on the
/// guest's pointer would leave the upper half of the guest's `tv_usec` as it
/// was: whatever the guest's stack held there.
fn gettimeofday(tv: u64, tz: u64) -> i64 {
    let Some(ns) = crate::clock::now_ns(crate::clock::REALTIME) else {
        return EINVAL;
    };
    if tv != 0 {
        // SAFETY: the guest passes a Linux struct timeval.
        unsafe {
            std::ptr::write_unaligned(
                tv as *mut [i64; 2],
                [
                    (ns / crate::clock::NANOS) as i64,
                    (ns % crate::clock::NANOS / 1000) as i64,
                ],
            );
        }
    }
    if tz != 0 {
        // SAFETY: the guest passes a struct timezone, two ints; UTC, as Linux
        // answers.
        unsafe { std::ptr::write_unaligned(tz as *mut [i32; 2], [0, 0]) };
    }
    0
}

/// `tgkill` and `kill` sent to this process: SIGABRT aborts, signal 0 probes.
fn kill(signal: u64) -> i64 {
    match signal {
        0 => 0,
        6 => std::process::abort(),
        _ => {
            eprintln!("cs2metal: the guest sent itself signal {signal}; ignored");
            0
        }
    }
}

/// The host's number for a Linux signal a process may send another, or `None`.
fn host_signal(linux: u64) -> Option<libc::c_int> {
    Some(match linux {
        1 => libc::SIGHUP,
        2 => libc::SIGINT,
        3 => libc::SIGQUIT,
        9 => libc::SIGKILL,
        10 => libc::SIGUSR1,
        12 => libc::SIGUSR2,
        15 => libc::SIGTERM,
        17 => libc::SIGCHLD,
        18 => libc::SIGCONT,
        19 => libc::SIGSTOP,
        _ => return None,
    })
}

/// `kill(pid, signal)`. To this process (its pid or 0), as [`kill`]; to another,
/// through the host, so that signal 0 tells whether that process is alive. A
/// program that finds a lock or semaphore left by an earlier instance asks
/// exactly that.
fn kill_process(pid: u64, signal: u64) -> i64 {
    let pid = pid as i32;
    // SAFETY: getpid has no preconditions.
    let own = unsafe { libc::getpid() };
    if pid == own || pid == 0 {
        return kill(signal);
    }
    let host = if signal == 0 {
        0
    } else {
        match host_signal(signal) {
            Some(s) => s,
            None => return EINVAL,
        }
    };
    // SAFETY: kill only delivers a signal or probes a pid.
    result(i64::from(unsafe { libc::kill(pid, host) }))
}

/// Handles the system calls of this module.
pub(crate) fn answer(f: &SyscallFrame) -> Option<i64> {
    let (a0, a1, a2, a3) = (f.arg0, f.arg1, f.arg2, f.arg3);
    if let Some(value) = crate::ipc::answer(f.number, [a0, a1, a2, a3]) {
        return Some(value);
    }
    let fd = a0 as libc::c_int;
    // SAFETY: every call below passes the guest's own arguments to libSystem, as
    // the Linux kernel would use them, in the guest's address space.
    let value = unsafe {
        match f.number {
            0 => return fds::read(fd, a1, a2),
            1 => return fds::write(fd, a1, a2),
            7 => result(i64::from(libc::poll(
                a0 as *mut libc::pollfd,
                a1 as libc::nfds_t,
                a2 as i32,
            ))),
            22 => pipe2(a0, 0),
            25 => mremap(a0, a1, a2, a3),
            32 => result(i64::from(libc::dup(fd))),
            33 => result(i64::from(libc::dup2(fd, a1 as libc::c_int))),
            35 => sleep(a0),
            62 => kill_process(a0, a1),
            // flock: the same operations (LOCK_SH, LOCK_EX, LOCK_NB, LOCK_UN).
            73 => result(i64::from(libc::flock(fd, a1 as libc::c_int))),
            74 | 75 => result(i64::from(libc::fsync(fd))),
            77 => result(i64::from(libc::ftruncate(fd, a1 as libc::off_t))),
            80 => match host_path(a0, libc::AT_FDCWD, Use::Read) {
                Ok(p) => result(i64::from(libc::chdir(p.as_ptr()))),
                Err(e) => e,
            },
            81 => result(i64::from(libc::fchdir(fd))),
            82 => match (
                host_path(a0, libc::AT_FDCWD, Use::Modify),
                host_path(a1, libc::AT_FDCWD, Use::Replace),
            ) {
                (Ok(from), Ok(to)) => result(i64::from(libc::rename(from.as_ptr(), to.as_ptr()))),
                (Err(e), _) | (_, Err(e)) => e,
            },
            83 => match host_path(a0, libc::AT_FDCWD, Use::Replace) {
                Ok(p) => result(i64::from(libc::mkdir(p.as_ptr(), a1 as libc::mode_t))),
                Err(e) => e,
            },
            84 => match host_path(a0, libc::AT_FDCWD, Use::Remove) {
                Ok(p) => result(i64::from(libc::rmdir(p.as_ptr()))),
                Err(e) => e,
            },
            // link: the source as Modify, so that an installation file is copied
            // up first and no link ever shares the installation's inode.
            86 => match (
                host_path(a0, libc::AT_FDCWD, Use::Modify),
                host_path(a1, libc::AT_FDCWD, Use::Replace),
            ) {
                (Ok(from), Ok(to)) => result(i64::from(libc::link(from.as_ptr(), to.as_ptr()))),
                (Err(e), _) | (_, Err(e)) => e,
            },
            87 => match host_path(a0, libc::AT_FDCWD, Use::Remove) {
                Ok(p) => result(i64::from(libc::unlink(p.as_ptr()))),
                Err(e) => e,
            },
            90 => match host_path(a0, libc::AT_FDCWD, Use::Modify) {
                Ok(p) => result(i64::from(libc::chmod(p.as_ptr(), a1 as libc::mode_t))),
                Err(e) => e,
            },
            91 => {
                if crate::paths::is_installation_file(fd) {
                    -30 // EROFS: the installation is read-only to the guest
                } else {
                    result(i64::from(libc::fchmod(fd, a1 as libc::mode_t)))
                }
            }
            95 => i64::from(libc::umask(a0 as libc::mode_t)),
            96 => gettimeofday(a0, a1),
            97 => {
                // getrlimit: the same answers as prlimit64.
                if a1 != 0 {
                    std::ptr::write_unaligned(a1 as *mut [u64; 2], [u64::MAX, u64::MAX]);
                }
                0
            }
            98 => {
                // getrusage: zero usage; the engine only reads it for statistics.
                if a1 != 0 {
                    std::ptr::write_bytes(a1 as *mut u8, 0, 144);
                }
                0
            }
            99 => sysinfo(a0),
            110 => i64::from(libc::getppid()),
            137 | 138 => {
                let mut st: libc::statfs = std::mem::zeroed();
                let status = if f.number == 137 {
                    match host_path(a0, libc::AT_FDCWD, Use::Read) {
                        Ok(p) => libc::statfs(p.as_ptr(), &mut st),
                        Err(e) => return Some(e),
                    }
                } else {
                    libc::fstatfs(fd, &mut st)
                };
                if status != 0 {
                    result(-1)
                } else {
                    statfs(&st, a1)
                }
            }
            140 => 20, // getpriority: nice 0, as the kernel encodes it
            141 | 144 | 149 | 150 | 160 | 203 => 0, // setpriority, sched_setscheduler,
            // mlock, munlock, setrlimit, sched_setaffinity: accepted as hints
            157 => crate::threads::prctl(a0, a1),
            146 => 99, // sched_get_priority_max
            147 => 0,  // sched_get_priority_min
            201 => {
                let wall = crate::clock::now_ns(crate::clock::REALTIME).unwrap_or(0);
                let now = (wall / crate::clock::NANOS) as i64;
                if a0 != 0 {
                    std::ptr::write_unaligned(a0 as *mut i64, now);
                }
                now
            }
            217 => fds::getdents64(fd, a1, a2),
            230 => clock_nanosleep(a0, a1, a2),
            // tgkill: threads of this process only.
            234 => kill(a2),
            258 => match host_path(a1, dirfd(a0 as i64), Use::Replace) {
                Ok(p) => result(i64::from(libc::mkdirat(
                    dirfd(a0 as i64),
                    p.as_ptr(),
                    a2 as libc::mode_t,
                ))),
                Err(e) => e,
            },
            263 => match host_path(a1, dirfd(a0 as i64), Use::Remove) {
                Ok(p) => {
                    let flags = if a2 & 0x200 != 0 {
                        libc::AT_REMOVEDIR
                    } else {
                        0
                    };
                    result(i64::from(libc::unlinkat(
                        dirfd(a0 as i64),
                        p.as_ptr(),
                        flags,
                    )))
                }
                Err(e) => e,
            },
            264 | 316 => match (
                host_path(a1, dirfd(a0 as i64), Use::Modify),
                host_path(a3, dirfd(a2 as i64), Use::Replace),
            ) {
                (Ok(from), Ok(to)) => result(i64::from(libc::renameat(
                    dirfd(a0 as i64),
                    from.as_ptr(),
                    dirfd(a2 as i64),
                    to.as_ptr(),
                ))),
                (Err(e), _) | (_, Err(e)) => e,
            },
            265 => match (
                host_path(a1, dirfd(a0 as i64), Use::Modify),
                host_path(a3, dirfd(a2 as i64), Use::Replace),
            ) {
                (Ok(from), Ok(to)) => result(i64::from(libc::linkat(
                    dirfd(a0 as i64),
                    from.as_ptr(),
                    dirfd(a2 as i64),
                    to.as_ptr(),
                    if f.arg4 & 0x400 != 0 {
                        libc::AT_SYMLINK_FOLLOW
                    } else {
                        0
                    },
                ))),
                (Err(e), _) | (_, Err(e)) => e,
            },
            267 if a1 != 0
                && CStr::from_ptr(a1 as *const c_char)
                    .to_bytes()
                    .starts_with(b"/proc/") =>
            {
                crate::syscall::readlink(a1, a2, a3)
            }
            267 => match host_path(a1, dirfd(a0 as i64), Use::Read) {
                Ok(p) => result(libc::readlinkat(
                    dirfd(a0 as i64),
                    p.as_ptr(),
                    a2 as *mut c_char,
                    a3 as usize,
                ) as i64),
                Err(e) => e,
            },
            268 => match host_path(a1, dirfd(a0 as i64), Use::Modify) {
                Ok(p) => result(i64::from(libc::fchmodat(
                    dirfd(a0 as i64),
                    p.as_ptr(),
                    a2 as libc::mode_t,
                    0,
                ))),
                Err(e) => e,
            },
            // utimes: struct timeval[2], or null for now; the host's timeval holds
            // the microseconds in the low half of Linux's 64-bit field.
            235 => match host_path(a0, libc::AT_FDCWD, Use::Modify) {
                Ok(p) => {
                    let times = if a1 == 0 {
                        None
                    } else {
                        let [s0, u0, s1, u1] = std::ptr::read_unaligned(a1 as *const [i64; 4]);
                        Some([
                            libc::timeval {
                                tv_sec: s0,
                                tv_usec: u0 as i32,
                            },
                            libc::timeval {
                                tv_sec: s1,
                                tv_usec: u1 as i32,
                            },
                        ])
                    };
                    result(i64::from(libc::utimes(
                        p.as_ptr(),
                        times.as_ref().map_or(std::ptr::null(), |t| t.as_ptr()),
                    )))
                }
                Err(e) => e,
            },
            269 => match host_path(a1, dirfd(a0 as i64), Use::Read) {
                Ok(p) => result(i64::from(libc::faccessat(
                    dirfd(a0 as i64),
                    p.as_ptr(),
                    a2 as libc::c_int,
                    0,
                ))),
                Err(e) => e,
            },
            271 => {
                // ppoll: a timespec timeout; the signal mask is not emulated.
                let timeout = if a2 == 0 {
                    -1
                } else {
                    let [s, ns] = std::ptr::read_unaligned(a2 as *const [i64; 2]);
                    (s * 1000 + ns / 1_000_000) as i32
                };
                result(i64::from(libc::poll(
                    a0 as *mut libc::pollfd,
                    a1 as libc::nfds_t,
                    timeout,
                )))
            }
            37 => 0, // alarm: no SIGALRM is delivered yet
            143 => {
                // sched_getparam: priority 0 under SCHED_OTHER.
                if a1 != 0 {
                    std::ptr::write_unaligned(a1 as *mut i32, 0);
                }
                0
            }
            145 => 0,                       // sched_getscheduler: SCHED_OTHER
            213 => crate::epoll::create(0), // epoll_create
            221 => 0,                       // fadvise64: advice only
            232 => crate::epoll::wait(a0 as i64, a1, a2, a3 as i32 as i64),
            233 => crate::epoll::ctl(a0 as i64, a1, a2 as i64, a3),
            281 => crate::epoll::wait(a0 as i64, a1, a2, a3 as i32 as i64), // epoll_pwait
            291 => crate::epoll::create(a0),
            284 => fds::eventfd(a0, 0),
            290 => fds::eventfd(a0, a1),
            292 => result(i64::from(libc::dup2(fd, a1 as libc::c_int))),
            293 => pipe2(a0, a1),
            _ => return None,
        }
    };
    Some(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The guest's buffer starts as stack garbage would: all ones.
    #[test]
    fn gettimeofday_writes_all_of_linux_timeval() {
        let mut tv = [-1i64; 2];
        let mut tz = [-1i32; 2];
        let status = gettimeofday(
            std::ptr::from_mut(&mut tv) as u64,
            std::ptr::from_mut(&mut tz) as u64,
        );
        assert_eq!(status, 0);
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_secs() as i64;
        assert!((now - 2..=now).contains(&tv[0]), "seconds {}", tv[0]);
        assert!((0..1_000_000).contains(&tv[1]), "microseconds {}", tv[1]);
        assert_eq!(tz, [0, 0]);
    }
}
