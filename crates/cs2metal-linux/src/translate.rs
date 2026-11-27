//! Linux values on the guest side, macOS values on the host side.

/// Linux errno for a macOS errno. Values 1 to 34 are the same on both.
pub fn errno(mac: i32) -> i64 {
    let linux = match mac {
        1..=34 => mac,
        35 => 11,  // EAGAIN
        36 => 115, // EINPROGRESS
        37 => 114, // EALREADY
        38 => 88,  // ENOTSOCK
        39 => 89,  // EDESTADDRREQ
        40 => 90,  // EMSGSIZE
        41 => 91,  // EPROTOTYPE
        42 => 92,  // ENOPROTOOPT
        43 => 93,  // EPROTONOSUPPORT
        45 => 95,  // EOPNOTSUPP
        47 => 97,  // EAFNOSUPPORT
        48 => 98,  // EADDRINUSE
        49 => 99,  // EADDRNOTAVAIL
        50 => 100, // ENETDOWN
        51 => 101, // ENETUNREACH
        53 => 103, // ECONNABORTED
        54 => 104, // ECONNRESET
        55 => 105, // ENOBUFS
        56 => 106, // EISCONN
        57 => 107, // ENOTCONN
        60 => 110, // ETIMEDOUT
        61 => 111, // ECONNREFUSED
        62 => 40,  // ELOOP
        63 => 36,  // ENAMETOOLONG
        64 => 112, // EHOSTDOWN
        65 => 113, // EHOSTUNREACH
        66 => 39,  // ENOTEMPTY
        69 => 122, // EDQUOT
        78 => 38,  // ENOSYS
        84 => 75,  // EOVERFLOW
        _ => 5,    // EIO, for anything without a counterpart
    };
    -i64::from(linux)
}

/// The result of a host call that returns -1 and sets errno, as Linux returns it.
pub fn result(value: i64) -> i64 {
    if value < 0 {
        errno(std::io::Error::last_os_error().raw_os_error().unwrap_or(5))
    } else {
        value
    }
}

pub const EBADF: i64 = -9;
pub const EAGAIN: i64 = -11;
pub const ENOMEM: i64 = -12;
pub const EACCES: i64 = -13;
pub const EFAULT: i64 = -14;
pub const EINVAL: i64 = -22;
pub const ENOTTY: i64 = -25;
pub const ERANGE: i64 = -34;
pub const ENOSYS: i64 = -38;
pub const ETIMEDOUT: i64 = -110;

/// `AT_FDCWD`, which Linux spells -100 and macOS -2.
pub fn dirfd(linux: i64) -> libc::c_int {
    if linux as i32 == -100 {
        libc::AT_FDCWD
    } else {
        linux as libc::c_int
    }
}

/// macOS `open` flags for Linux ones.
pub fn open_flags(linux: u64) -> libc::c_int {
    let mut mac = (linux & 0b11) as libc::c_int; // O_RDONLY, O_WRONLY, O_RDWR
    for (l, m) in [
        (0o100, libc::O_CREAT),
        (0o200, libc::O_EXCL),
        (0o400, libc::O_NOCTTY),
        (0o1000, libc::O_TRUNC),
        (0o2000, libc::O_APPEND),
        (0o4000, libc::O_NONBLOCK),
        (0o200000, libc::O_DIRECTORY),
        (0o400000, libc::O_NOFOLLOW),
        (0o2000000, libc::O_CLOEXEC),
    ] {
        if linux & l != 0 {
            mac |= m;
        }
    }
    mac
}

/// Linux `open` flags for macOS ones, for `fcntl(F_GETFL)`.
pub fn linux_open_flags(mac: libc::c_int) -> u64 {
    let mut linux = (mac & 0b11) as u64;
    for (l, m) in [(0o2000, libc::O_APPEND), (0o4000, libc::O_NONBLOCK)] {
        if mac & m != 0 {
            linux |= l;
        }
    }
    linux
}

/// macOS `mmap` flags for Linux ones: `MAP_ANONYMOUS` moves; the rest that macOS
/// lacks (`MAP_DENYWRITE`, `MAP_NORESERVE`, `MAP_STACK`, `MAP_POPULATE`) only
/// advise, and are dropped.
pub fn mmap_flags(linux: u64) -> libc::c_int {
    let mut mac = (linux & 0x3) as libc::c_int; // MAP_SHARED, MAP_PRIVATE
    if linux & 0x10 != 0 || linux & 0x10_0000 != 0 {
        mac |= libc::MAP_FIXED; // MAP_FIXED, MAP_FIXED_NOREPLACE
    }
    if linux & 0x20 != 0 {
        mac |= libc::MAP_ANON;
    }
    mac
}

/// Linux `struct stat` on x86-64: 144 bytes.
#[repr(C)]
#[derive(Debug, Default, Clone, Copy)]
pub struct LinuxStat {
    pub dev: u64,
    pub ino: u64,
    pub nlink: u64,
    pub mode: u32,
    pub uid: u32,
    pub gid: u32,
    pub pad0: u32,
    pub rdev: u64,
    pub size: i64,
    pub blksize: i64,
    pub blocks: i64,
    pub atime: i64,
    pub atime_nsec: i64,
    pub mtime: i64,
    pub mtime_nsec: i64,
    pub ctime: i64,
    pub ctime_nsec: i64,
    pub reserved: [i64; 3],
}

impl From<&libc::stat> for LinuxStat {
    fn from(s: &libc::stat) -> Self {
        Self {
            dev: s.st_dev as u64,
            ino: s.st_ino,
            nlink: u64::from(s.st_nlink),
            mode: u32::from(s.st_mode),
            uid: s.st_uid,
            gid: s.st_gid,
            pad0: 0,
            rdev: s.st_rdev as u64,
            size: s.st_size,
            blksize: i64::from(s.st_blksize),
            blocks: s.st_blocks,
            atime: s.st_atime,
            atime_nsec: s.st_atime_nsec,
            mtime: s.st_mtime,
            mtime_nsec: s.st_mtime_nsec,
            ctime: s.st_ctime,
            ctime_nsec: s.st_ctime_nsec,
            reserved: [0; 3],
        }
    }
}

/// The macOS clock for a Linux clock id.
pub fn clock(linux: u64) -> Option<libc::clockid_t> {
    Some(match linux {
        0 | 5 => libc::CLOCK_REALTIME,      // REALTIME, REALTIME_COARSE
        1 | 6 | 7 => libc::CLOCK_MONOTONIC, // MONOTONIC, MONOTONIC_COARSE, BOOTTIME
        2 => libc::CLOCK_PROCESS_CPUTIME_ID,
        3 => libc::CLOCK_THREAD_CPUTIME_ID,
        4 => libc::CLOCK_MONOTONIC_RAW,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn translates_errno_open_and_mmap_flags() {
        assert_eq!(errno(libc::ENOENT), -2);
        assert_eq!(errno(libc::EAGAIN), -11);
        assert_eq!(errno(libc::ENOSYS), -38);
        assert_eq!(errno(libc::ENAMETOOLONG), -36);
        // O_RDONLY | O_CLOEXEC, as ld.so opens libraries.
        assert_eq!(open_flags(0o2000000), libc::O_RDONLY | libc::O_CLOEXEC);
        assert_eq!(
            open_flags(0o1 | 0o100 | 0o1000),
            libc::O_WRONLY | libc::O_CREAT | libc::O_TRUNC
        );
        // MAP_PRIVATE | MAP_ANONYMOUS, and MAP_PRIVATE | MAP_FIXED | MAP_DENYWRITE.
        assert_eq!(mmap_flags(0x22), libc::MAP_PRIVATE | libc::MAP_ANON);
        assert_eq!(mmap_flags(0x812), libc::MAP_PRIVATE | libc::MAP_FIXED);
        assert_eq!(dirfd(-100), libc::AT_FDCWD);
        assert_eq!(std::mem::size_of::<LinuxStat>(), 144);
    }
}
