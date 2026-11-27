//! Linux system calls, answered with macOS services in the host domain.
//!
//! The handler runs on the guest thread's own stack, after the system call entry
//! has switched GS to the host. Guest pointers are used as the kernel would use
//! them, in the guest's own address space, which is this process.

use std::cell::RefCell;
use std::collections::BTreeSet;
use std::ffi::{CStr, c_char};
use std::sync::Mutex;

use cs2metal_loader::{HostBlock, SyscallFrame, attach};

use crate::mmap;
use crate::translate::{
    self, EBADF, EFAULT, EINVAL, ENOSYS, ENOTTY, ERANGE, LinuxStat, dirfd, open_flags, result,
};

thread_local! {
    /// The host block of the guest thread running on this host thread.
    pub(crate) static BLOCK: RefCell<Option<HostBlock>> = const { RefCell::new(None) };
}

/// The program `/proc/self/exe` names.
pub(crate) static EXE: Mutex<String> = Mutex::new(String::new());

static UNSUPPORTED: Mutex<BTreeSet<u64>> = Mutex::new(BTreeSet::new());

/// A guest C string. A null pointer reads as empty.
///
/// # Safety
///
/// `p` must be null or point at a NUL-terminated string in guest memory.
unsafe fn guest_str<'a>(p: u64) -> Option<&'a CStr> {
    // SAFETY: the caller's contract.
    (p != 0).then(|| unsafe { CStr::from_ptr(p as *const c_char) })
}

/// A guest path as the host should use it (the shims' paths, the installation
/// overlay), or a Linux errno.
///
/// # Safety
///
/// `p` must be null or point at a NUL-terminated string in guest memory.
pub(crate) unsafe fn host_path(
    p: u64,
    dirfd: libc::c_int,
    how: crate::paths::Use,
) -> Result<std::borrow::Cow<'static, CStr>, i64> {
    // SAFETY: the caller's contract.
    let path: &'static CStr = unsafe { guest_str(p) }.ok_or(EFAULT)?;
    crate::paths::resolve(path, dirfd, how)
}

fn write_stat(out: u64, st: &libc::stat) -> i64 {
    if out == 0 {
        return EFAULT;
    }
    let linux = LinuxStat::from(st);
    // SAFETY: the guest passed a buffer for a Linux struct stat.
    unsafe { std::ptr::write_unaligned(out as *mut LinuxStat, linux) };
    0
}

fn stat_path(path: u64, out: u64, follow: bool) -> i64 {
    // SAFETY: the guest passes a path string.
    let path = match unsafe { host_path(path, libc::AT_FDCWD, crate::paths::Use::Read) } {
        Ok(p) => p,
        Err(e) => return e,
    };
    // SAFETY: stat writes into the zeroed struct.
    let mut st: libc::stat = unsafe { std::mem::zeroed() };
    // SAFETY: path is NUL-terminated; st is a valid out pointer.
    let status = unsafe {
        if follow {
            libc::stat(path.as_ptr(), &mut st)
        } else {
            libc::lstat(path.as_ptr(), &mut st)
        }
    };
    if status != 0 {
        return result(-1);
    }
    write_stat(out, &st)
}

fn fstat(fd: i64, out: u64) -> i64 {
    // SAFETY: fstat writes into the zeroed struct.
    let mut st: libc::stat = unsafe { std::mem::zeroed() };
    // SAFETY: as above.
    if unsafe { libc::fstat(fd as libc::c_int, &mut st) } != 0 {
        return result(-1);
    }
    write_stat(out, &st)
}

/// `arch_prctl(ARCH_SET_FS)` and `ARCH_GET_FS`: the guest's thread pointer, which
/// under T-001 is the GS base the bridge returns to.
fn arch_prctl(code: u64, addr: u64) -> i64 {
    const ARCH_SET_FS: u64 = 0x1002;
    const ARCH_GET_FS: u64 = 0x1003;
    BLOCK.with(|b| {
        let block = b.borrow();
        let Some(block) = block.as_ref() else {
            return EINVAL;
        };
        match code {
            ARCH_SET_FS => {
                // SAFETY: glibc passes its new thread control block, at least a
                // tcbhead_t long; the slot and the host block pointer go inside it.
                unsafe { attach(addr, block) };
                match block.set_guest_tp(addr) {
                    Ok(()) => 0,
                    Err(_) => EINVAL,
                }
            }
            ARCH_GET_FS if addr != 0 => {
                // SAFETY: the guest passes a pointer to an unsigned long.
                unsafe { std::ptr::write_unaligned(addr as *mut u64, block.guest_tp()) };
                0
            }
            _ => EINVAL,
        }
    })
}

/// `struct utsname` with Linux's field size of 65 bytes.
fn uname(out: u64) -> i64 {
    if out == 0 {
        return EFAULT;
    }
    let mut host = [0u8; 256];
    // SAFETY: gethostname writes at most the buffer's length.
    unsafe { libc::gethostname(host.as_mut_ptr().cast(), host.len()) };
    let host_len = host.iter().position(|&b| b == 0).unwrap_or(0).min(64);
    let fields: [&[u8]; 6] = [
        b"Linux",
        &host[..host_len],
        b"5.15.0-cs2metal",
        b"#1 SMP",
        b"x86_64",
        b"(none)",
    ];
    for (i, f) in fields.iter().enumerate() {
        let at = out + 65 * i as u64;
        // SAFETY: the guest passed a 390-byte struct utsname; each field is 65
        // bytes and gets at most 64 plus a NUL.
        unsafe {
            std::ptr::write_bytes(at as *mut u8, 0, 65);
            std::ptr::copy_nonoverlapping(f.as_ptr(), at as *mut u8, f.len().min(64));
        }
    }
    0
}

/// Whether a guest path names this process's `/proc` executable link.
fn is_own_exe(path: &[u8]) -> bool {
    let pid = std::process::id().to_string();
    path == b"/proc/self/exe"
        || path == b"/proc/thread-self/exe"
        || path == format!("/proc/{pid}/exe").as_bytes()
}

pub(crate) fn readlink(path: u64, buf: u64, size: u64) -> i64 {
    // SAFETY: the guest passes a path string.
    let Some(path) = (unsafe { guest_str(path) }) else {
        return EFAULT;
    };
    if is_own_exe(path.to_bytes()) {
        let exe = EXE.lock().map(|e| e.clone()).unwrap_or_default();
        let n = exe.len().min(size as usize);
        // SAFETY: the guest passed a buffer of `size` bytes.
        unsafe { std::ptr::copy_nonoverlapping(exe.as_ptr(), buf as *mut u8, n) };
        return n as i64;
    }
    // SAFETY: path is NUL-terminated; buf has size bytes.
    result(unsafe { libc::readlink(path.as_ptr(), buf as *mut c_char, size as usize) } as i64)
}

fn prlimit64(resource: u64, new: u64, old: u64) -> i64 {
    const RLIMIT_STACK: u64 = 3;
    const RLIMIT_NOFILE: u64 = 7;
    let _ = new; // limits are not changed
    if old != 0 {
        let (cur, max) = match resource {
            RLIMIT_STACK => (8 << 20, u64::MAX),
            RLIMIT_NOFILE => (10240, 10240),
            _ => (u64::MAX, u64::MAX),
        };
        // SAFETY: the guest passes a struct rlimit64 of two u64 words.
        unsafe {
            std::ptr::write_unaligned(old as *mut [u64; 2], [cur, max]);
        }
    }
    0
}

fn clock_gettime(id: u64, out: u64) -> i64 {
    let Some(clock) = translate::clock(id) else {
        return EINVAL;
    };
    // SAFETY: struct timespec has the same layout on both systems on x86-64.
    result(i64::from(unsafe {
        libc::clock_gettime(clock, out as *mut libc::timespec)
    }))
}

fn getcwd(buf: u64, size: u64) -> i64 {
    // SAFETY: the guest passed a buffer of `size` bytes.
    let p = unsafe { libc::getcwd(buf as *mut c_char, size as usize) };
    if p.is_null() {
        return result(-1);
    }
    // SAFETY: getcwd wrote a NUL-terminated string there.
    let len = unsafe { CStr::from_ptr(p) }.to_bytes().len();
    if len + 1 > size as usize {
        return ERANGE;
    }
    len as i64 + 1
}

fn unsupported(number: u64) -> i64 {
    if let Ok(mut seen) = UNSUPPORTED.lock() {
        if seen.insert(number) {
            eprintln!("cs2metal: unsupported Linux system call {number}");
        }
    }
    ENOSYS
}

/// Whether `CS2METAL_TRACE` asks for a line per system call on standard error.
fn tracing() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("CS2METAL_TRACE").is_some())
}

/// The system call handler the bridge runs.
pub(crate) extern "C" fn handle(f: &mut SyscallFrame) -> u64 {
    let traced = tracing().then_some((f.number, [f.arg0, f.arg1, f.arg2, f.arg3, f.arg4, f.arg5]));
    if let Some((n, a)) = traced {
        eprintln!(
            "cs2metal: syscall {n}({:#x}, {:#x}, {:#x}, {:#x}, {:#x}, {:#x})",
            a[0], a[1], a[2], a[3], a[4], a[5]
        );
    }
    let value = answer(f);
    if traced.is_some() {
        eprintln!("cs2metal:   = {value:#x}");
    }
    value
}

/// `fcntl` record locks with Linux's `struct flock` (type and whence first, 32
/// bytes), which macOS lays out differently (start, length, pid, then type and
/// whence, 24 bytes), and Linux's lock type numbers.
fn record_lock(fd: libc::c_int, command: u64, lock: u64) -> i64 {
    if lock == 0 {
        return EFAULT;
    }
    // SAFETY: the guest passes a struct flock of 32 bytes.
    let (kind, whence, start, len) = unsafe {
        (
            std::ptr::read_unaligned(lock as *const i16),
            std::ptr::read_unaligned((lock + 2) as *const i16),
            std::ptr::read_unaligned((lock + 8) as *const i64),
            std::ptr::read_unaligned((lock + 16) as *const i64),
        )
    };
    let kind = match kind {
        0 => libc::F_RDLCK,
        1 => libc::F_WRLCK,
        2 => libc::F_UNLCK,
        _ => return EINVAL,
    };
    // SAFETY: flock is plain data.
    let mut host: libc::flock = unsafe { std::mem::zeroed() };
    host.l_type = kind;
    host.l_whence = whence;
    host.l_start = start;
    host.l_len = len;
    let host_command = match command {
        5 | 36 => libc::F_GETLK,
        6 | 37 => libc::F_SETLK,
        _ => libc::F_SETLKW,
    };
    // SAFETY: fcntl with a host struct flock.
    if unsafe { libc::fcntl(fd, host_command, &mut host) } < 0 {
        return result(-1);
    }
    if host_command == libc::F_GETLK {
        let kind: i16 = match host.l_type {
            libc::F_RDLCK => 0,
            libc::F_WRLCK => 1,
            _ => 2,
        };
        // SAFETY: as above, writing the guest's struct flock.
        unsafe {
            std::ptr::write_unaligned(lock as *mut i16, kind);
            std::ptr::write_unaligned((lock + 2) as *mut i16, host.l_whence);
            std::ptr::write_unaligned((lock + 8) as *mut i64, host.l_start);
            std::ptr::write_unaligned((lock + 16) as *mut i64, host.l_len);
            std::ptr::write_unaligned((lock + 24) as *mut i32, host.l_pid);
        }
    }
    0
}

fn answer(f: &mut SyscallFrame) -> u64 {
    if f.number == crate::steam::BIND_STEAM {
        return crate::steam::bind(f.arg0, f.arg1, f.arg2) as u64;
    }
    if f.number == crate::sdl::BIND_SDL {
        return crate::sdl::bind(f.arg0, f.arg1, f.arg2) as u64;
    }
    if f.number == crate::vulkan::BIND_VULKAN {
        return crate::vulkan::bind(f.arg0, f.arg1, f.arg2) as u64;
    }
    if let Some(value) = crate::net::answer(f) {
        return value as u64;
    }
    if let Some(value) = crate::more::answer(f) {
        return value as u64;
    }
    let (a0, a1, a2, a3) = (f.arg0, f.arg1, f.arg2, f.arg3);
    let fd = a0 as libc::c_int;
    // SAFETY: every call below passes the guest's own arguments to libSystem, as
    // the Linux kernel would use them, in the guest's address space.
    let value: i64 = unsafe {
        match f.number {
            0 => result(libc::read(fd, a1 as *mut _, a2 as usize) as i64),
            1 => result(libc::write(fd, a1 as *const _, a2 as usize) as i64),
            2 => match host_path(a0, libc::AT_FDCWD, crate::paths::open_use(a1)) {
                Ok(p) => result(i64::from(libc::open(
                    p.as_ptr(),
                    open_flags(a1),
                    a2 as libc::c_uint,
                ))),
                Err(e) => e,
            },
            3 => {
                crate::fds::close(fd);
                crate::fds::close_dir(fd);
                crate::epoll::close(a0 as i64);
                result(i64::from(libc::close(fd)))
            }
            4 => stat_path(a0, a1, true),
            5 => fstat(a0 as i64, a1),
            6 => stat_path(a0, a1, false),
            8 => result(libc::lseek(fd, a1 as libc::off_t, a2 as libc::c_int)),
            9 => mmap::mmap(a0, a1, a2, a3, f.arg4 as i64, f.arg5),
            10 => result(i64::from(libc::mprotect(
                a0 as *mut _,
                a1 as usize,
                (a2 & 7) as libc::c_int,
            ))),
            11 => result(i64::from(libc::munmap(a0 as *mut _, a1 as usize))),
            12 => crate::brk::brk(a0),
            13 | 14 => {
                // rt_sigaction, rt_sigprocmask: recorded as done; signals come
                // with their own slice.
                if a2 != 0 {
                    std::ptr::write_bytes(a2 as *mut u8, 0, a3.min(128) as usize);
                }
                0
            }
            16 => ENOTTY, // ioctl: no terminal emulation yet
            17 => result(libc::pread(fd, a1 as *mut _, a2 as usize, a3 as libc::off_t) as i64),
            20 => result(libc::writev(fd, a1 as *const libc::iovec, a2 as libc::c_int) as i64),
            21 => match host_path(a0, libc::AT_FDCWD, crate::paths::Use::Read) {
                Ok(p) => result(i64::from(libc::access(p.as_ptr(), a1 as libc::c_int))),
                Err(e) => e,
            },
            28 => 0, // madvise: advice only
            39 => i64::from(libc::getpid()),
            24 => i64::from(libc::sched_yield()),
            56 => crate::threads::clone(f),
            60 => crate::threads::exit_thread(a0),
            231 => {
                // exit_group ends the process, whatever thread calls it.
                let _ = std::io::Write::flush(&mut std::io::stdout());
                std::process::exit(a0 as i32)
            }
            63 => uname(a0),
            72 => match a1 {
                // F_DUPFD, F_GETFD, F_SETFD: the same numbers and values.
                0..=2 => result(i64::from(libc::fcntl(
                    fd,
                    a1 as libc::c_int,
                    a2 as libc::c_int,
                ))),
                1030 => result(i64::from(libc::fcntl(
                    fd,
                    libc::F_DUPFD_CLOEXEC,
                    a2 as libc::c_int,
                ))),
                // F_GETLK, F_SETLK, F_SETLKW; the open file description locks
                // (36 to 38) are answered as the process-associated ones.
                5..=7 | 36..=38 => record_lock(fd, a1, a2),
                4 => {
                    // F_SETFL: only O_APPEND and O_NONBLOCK change, as on Linux
                    // (O_ASYNC and O_DIRECT are not supported).
                    let flags = translate::open_flags(a2 & 0o6000);
                    crate::fds::set_nonblocking(fd, flags & libc::O_NONBLOCK != 0);
                    result(i64::from(libc::fcntl(fd, libc::F_SETFL, flags)))
                }
                3 => {
                    let flags = libc::fcntl(fd, libc::F_GETFL);
                    if flags < 0 {
                        result(-1)
                    } else {
                        translate::linux_open_flags(flags) as i64
                    }
                }
                _ => EINVAL,
            },
            79 => getcwd(a0, a1),
            89 => readlink(a0, a1, a2),
            102 | 107 => i64::from(libc::getuid()),
            104 | 108 => i64::from(libc::getgid()),
            158 => arch_prctl(a0, a1),
            186 => crate::threads::gettid(),
            202 => crate::threads::futex(a0, a1, a2, a3, f.arg4, f.arg5),
            218 => crate::threads::set_tid_address(a0),
            228 => clock_gettime(a0, a1),
            257 => match host_path(a1, dirfd(a0 as i64), crate::paths::open_use(a2)) {
                Ok(p) => result(i64::from(libc::openat(
                    dirfd(a0 as i64),
                    p.as_ptr(),
                    open_flags(a2),
                    a3 as libc::c_uint,
                ))),
                Err(e) => e,
            },
            262 => {
                // newfstatat: AT_EMPTY_PATH is fstat; AT_SYMLINK_NOFOLLOW is lstat.
                const AT_SYMLINK_NOFOLLOW: u64 = 0x100;
                const AT_EMPTY_PATH: u64 = 0x1000;
                let empty = guest_str(a1).is_none_or(|p| p.to_bytes().is_empty());
                if a3 & AT_EMPTY_PATH != 0 && empty {
                    fstat(a0 as i64, a2)
                } else if a0 as i32 == -100 {
                    stat_path(a1, a2, a3 & AT_SYMLINK_NOFOLLOW == 0)
                } else {
                    EBADF
                }
            }
            273 => 0, // set_robust_list: recorded as done; robust futexes are not emulated
            204 => {
                // sched_getaffinity: every CPU the host reports, as a mask.
                let n = std::thread::available_parallelism().map_or(1, |n| n.get());
                let bytes = (a1 as usize).min(128);
                std::ptr::write_bytes(a2 as *mut u8, 0, bytes);
                for cpu in 0..n.min(bytes * 8) {
                    *((a2 as *mut u8).add(cpu / 8)) |= 1 << (cpu % 8);
                }
                bytes as i64
            }
            302 => prlimit64(a1, a2, a3),
            318 => {
                libc::arc4random_buf(a0 as *mut _, a1 as usize);
                a1 as i64
            }
            number => unsupported(number),
        }
    };
    value as u64
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::CString;

    /// Answers system call `number` with Linux arguments, as the guest would make it.
    fn call(number: u64, args: [u64; 6]) -> i64 {
        let mut frame = SyscallFrame {
            number,
            arg0: args[0],
            arg1: args[1],
            arg2: args[2],
            arg3: args[3],
            arg4: args[4],
            arg5: args[5],
            saved_rbp: 0,
            saved_rbx: 0,
            return_address: 0,
        };
        answer(&mut frame) as i64
    }

    #[test]
    fn kill_probes_other_processes_on_the_host() {
        let mut child = std::process::Command::new("/bin/sleep")
            .arg("30")
            .spawn()
            .expect("child");
        let pid = u64::from(child.id());
        assert_eq!(call(62, [pid, 0, 0, 0, 0, 0]), 0, "alive");
        assert_eq!(call(62, [pid, 15, 0, 0, 0, 0]), 0, "SIGTERM");
        let _ = child.wait();
        assert_eq!(call(62, [pid, 0, 0, 0, 0, 0]), -3, "ESRCH once it is gone");
        let own = u64::from(std::process::id());
        assert_eq!(call(62, [own, 0, 0, 0, 0, 0]), 0, "this process");
    }

    #[test]
    fn flock_and_utimes_reach_the_file() {
        let path = std::env::temp_dir().join(format!("cs2metal-flock-{}", std::process::id()));
        let file = std::fs::File::create(&path).expect("file");
        let fd = std::os::fd::AsRawFd::as_raw_fd(&file) as u64;
        assert_eq!(call(73, [fd, 2 | 4, 0, 0, 0, 0]), 0, "LOCK_EX | LOCK_NB");
        assert_eq!(call(73, [fd, 8, 0, 0, 0, 0]), 0, "LOCK_UN");
        let name = CString::new(path.to_string_lossy().as_bytes()).expect("path");
        let times = [1_000_000_000i64, 0, 1_000_000_000, 0];
        assert_eq!(
            call(
                235,
                [name.as_ptr() as u64, times.as_ptr() as u64, 0, 0, 0, 0]
            ),
            0
        );
        let modified = std::fs::metadata(&path)
            .expect("stat")
            .modified()
            .expect("mtime");
        assert_eq!(
            modified,
            std::time::UNIX_EPOCH + std::time::Duration::from_secs(1_000_000_000)
        );
        drop(file);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn link_makes_a_second_name() {
        let dir = std::env::temp_dir().join(format!("cs2metal-link-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("dir");
        std::fs::write(dir.join("a"), b"x").expect("write");
        let c = |p: &str| CString::new(dir.join(p).to_string_lossy().as_bytes()).expect("path");
        let (a, b) = (c("a"), c("b"));
        assert_eq!(
            call(86, [a.as_ptr() as u64, b.as_ptr() as u64, 0, 0, 0, 0]),
            0
        );
        assert_eq!(std::fs::read(dir.join("b")).expect("read"), b"x");
        let at = c("c");
        let cwd = libc::AT_FDCWD as i64 as u64; // AT_FDCWD is -100 on both
        let args = [cwd, a.as_ptr() as u64, cwd, at.as_ptr() as u64, 0, 0];
        assert_eq!(call(265, args), 0, "linkat");
        assert!(dir.join("c").exists());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn fcntl_record_locks_use_linux_flock() {
        let path = std::env::temp_dir().join(format!("cs2metal-lock-{}", std::process::id()));
        let file = std::fs::File::create(&path).expect("file");
        let fd = std::os::fd::AsRawFd::as_raw_fd(&file) as u64;
        // Linux struct flock: F_WRLCK, SEEK_SET, start 0, length 0 (the whole file).
        let mut lock = [0u8; 32];
        lock[..2].copy_from_slice(&1i16.to_le_bytes());
        assert_eq!(
            call(72, [fd, 6, lock.as_ptr() as u64, 0, 0, 0]),
            0,
            "F_SETLK"
        );
        // The same process asking F_GETLK sees no conflict: F_UNLCK comes back.
        assert_eq!(
            call(72, [fd, 5, lock.as_mut_ptr() as u64, 0, 0, 0]),
            0,
            "F_GETLK"
        );
        assert_eq!(i16::from_le_bytes([lock[0], lock[1]]), 2);
        lock[..2].copy_from_slice(&9i16.to_le_bytes());
        assert_eq!(call(72, [fd, 6, lock.as_ptr() as u64, 0, 0, 0]), EINVAL);
        drop(file);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn fcntl_sets_status_flags_and_duplicates_close_on_exec() {
        let mut fds = [0 as libc::c_int; 2];
        // SAFETY: pipe writes two descriptors into the array.
        assert_eq!(unsafe { libc::pipe(fds.as_mut_ptr()) }, 0);
        let fd = fds[0] as u64;
        const O_NONBLOCK: u64 = 0o4000;
        assert_eq!(call(72, [fd, 4, O_NONBLOCK, 0, 0, 0]), 0, "F_SETFL");
        assert_eq!(
            call(72, [fd, 3, 0, 0, 0, 0]) as u64 & O_NONBLOCK,
            O_NONBLOCK
        );
        let copy = call(72, [fd, 1030, 100, 0, 0, 0]);
        assert!(copy >= 100, "F_DUPFD_CLOEXEC gave {copy}");
        assert_eq!(
            call(72, [copy as u64, 1, 0, 0, 0, 0]),
            1,
            "FD_CLOEXEC is set"
        );
        for f in [fds[0], fds[1], copy as libc::c_int] {
            // SAFETY: descriptors this test made.
            unsafe { libc::close(f) };
        }
    }
}
