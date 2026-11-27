//! Sockets: the BSD socket calls with Linux's layouts and numbers.
//!
//! macOS has the same socket calls; what differs is their data. A Linux socket
//! address starts with a 16-bit family where macOS has a length byte and a family
//! byte, and `AF_INET6` is 10 on Linux and 30 on macOS. Unix socket paths are 108
//! bytes long on Linux and 104 on macOS, and Linux has an abstract namespace, which
//! is mapped here onto files in a private directory. `msghdr` and `cmsghdr` have
//! different field widths, socket option levels and names differ, and so do some
//! message flags. Every socket made here gets `SO_NOSIGPIPE`, since the guest asks
//! for `MSG_NOSIGNAL` per call and macOS has no such flag.

use std::collections::HashMap;
use std::ffi::CString;
use std::sync::Mutex;

use cs2metal_loader::SyscallFrame;

use crate::paths::Use;
use crate::translate::{EAGAIN, EFAULT, EINVAL, errno, result};

const EAFNOSUPPORT: i64 = -97;
const ENOPROTOOPT: i64 = -92;
const ENAMETOOLONG: i64 = -36;

const SOCK_NONBLOCK: u64 = 0o4000;
const SOCK_CLOEXEC: u64 = 0o2000000;

const LINUX_AF_INET6: u16 = 10;
/// `sizeof(struct sockaddr_un)` on Linux, and its path's length.
const LINUX_SUN_LEN: usize = 110;
/// The length of `sun_path` on macOS.
const MAC_SUN_PATH: usize = 104;

/// A Linux socket domain on the host, or `None` for one macOS lacks (`AF_NETLINK`,
/// `AF_PACKET`).
fn host_domain(linux: u64) -> Option<libc::c_int> {
    match linux {
        0 => Some(libc::AF_UNSPEC),
        1 => Some(libc::AF_UNIX),
        2 => Some(libc::AF_INET),
        10 => Some(libc::AF_INET6),
        _ => None,
    }
}

/// Applies `SOCK_NONBLOCK` and `SOCK_CLOEXEC` from Linux type bits, and
/// `SO_NOSIGPIPE`, to a socket just made.
fn prepare(fd: libc::c_int, linux_type: u64) {
    let on: libc::c_int = 1;
    // SAFETY: fcntl and setsockopt on a descriptor this module just made.
    unsafe {
        if linux_type & SOCK_CLOEXEC != 0 {
            libc::fcntl(fd, libc::F_SETFD, libc::FD_CLOEXEC);
        }
        if linux_type & SOCK_NONBLOCK != 0 {
            let flags = libc::fcntl(fd, libc::F_GETFL);
            libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK);
        }
        libc::setsockopt(
            fd,
            libc::SOL_SOCKET,
            libc::SO_NOSIGPIPE,
            std::ptr::from_ref(&on).cast(),
            4,
        );
    }
}

// --- Addresses -------------------------------------------------------------------

/// Abstract Unix socket names by the host path that stands for them.
static ABSTRACT: Mutex<Option<HashMap<Vec<u8>, Vec<u8>>>> = Mutex::new(None);

/// The host path for a Linux abstract socket name (the bytes after its leading
/// NUL): a file in a directory private to this user, named by a hash of the name.
fn abstract_path(name: &[u8]) -> Vec<u8> {
    // FNV-1a: stable, and short enough for macOS's 104-byte paths.
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for &b in name {
        hash = (hash ^ u64::from(b)).wrapping_mul(0x0000_0100_0000_01b3);
    }
    // SAFETY: getuid cannot fail.
    let uid = unsafe { libc::getuid() };
    let dir = format!("/tmp/cs2metal-{uid}-abstract");
    let _ = std::fs::create_dir(&dir);
    let _ = std::fs::set_permissions(&dir, std::os::unix::fs::PermissionsExt::from_mode(0o700));
    let path = format!("{dir}/{hash:016x}").into_bytes();
    if let Ok(mut names) = ABSTRACT.lock() {
        names
            .get_or_insert_with(HashMap::new)
            .insert(path.clone(), name.to_vec());
    }
    path
}

fn abstract_name(path: &[u8]) -> Option<Vec<u8>> {
    ABSTRACT.lock().ok()?.as_ref()?.get(path).cloned()
}

/// A socket address in the host's layout.
struct HostAddr {
    storage: libc::sockaddr_storage,
    len: libc::socklen_t,
    /// A Unix socket path standing for an abstract name, which `bind` replaces.
    stands_for_abstract: bool,
}

impl HostAddr {
    fn empty() -> Self {
        Self {
            // SAFETY: sockaddr_storage is plain data.
            storage: unsafe { std::mem::zeroed() },
            len: std::mem::size_of::<libc::sockaddr_storage>() as libc::socklen_t,
            stands_for_abstract: false,
        }
    }

    fn bytes_mut(&mut self) -> &mut [u8] {
        // SAFETY: the storage is plain bytes of its own size.
        unsafe {
            std::slice::from_raw_parts_mut(
                std::ptr::from_mut(&mut self.storage).cast::<u8>(),
                std::mem::size_of::<libc::sockaddr_storage>(),
            )
        }
    }

    fn bytes(&self) -> &[u8] {
        // SAFETY: as above.
        unsafe {
            std::slice::from_raw_parts(
                std::ptr::from_ref(&self.storage).cast::<u8>(),
                std::mem::size_of::<libc::sockaddr_storage>(),
            )
        }
    }

    fn as_ptr(&self) -> *const libc::sockaddr {
        std::ptr::from_ref(&self.storage).cast()
    }

    fn as_mut_ptr(&mut self) -> *mut libc::sockaddr {
        std::ptr::from_mut(&mut self.storage).cast()
    }
}

/// A Linux socket address of `len` bytes at `ptr`, in the host's layout. Unix
/// socket paths go through the installation overlay with `how`.
fn addr_from_guest(ptr: u64, len: u64, how: Use) -> Result<HostAddr, i64> {
    if ptr == 0 {
        return Err(EFAULT);
    }
    let len = len as usize;
    if len < 2 {
        return Err(EINVAL);
    }
    // SAFETY: the guest passes `len` readable bytes; at most 128 are read.
    let guest = unsafe { std::slice::from_raw_parts(ptr as *const u8, len.min(128)) };
    let family = u16::from_le_bytes([guest[0], guest[1]]);
    let mut addr = HostAddr::empty();
    let out = addr.bytes_mut();
    let host_len = match family {
        2 => {
            if len < 16 {
                return Err(EINVAL);
            }
            out[2..16].copy_from_slice(&guest[2..16]);
            out[1] = libc::AF_INET as u8;
            16
        }
        LINUX_AF_INET6 => {
            if len < 24 {
                return Err(EINVAL);
            }
            let n = len.min(28);
            out[2..n].copy_from_slice(&guest[2..n]);
            out[1] = libc::AF_INET6 as u8;
            28
        }
        1 => {
            let raw = &guest[2..len.min(LINUX_SUN_LEN)];
            let path: Vec<u8> = if raw.first() == Some(&0) {
                addr.stands_for_abstract = true;
                abstract_path(&raw[1..])
            } else {
                let end = raw.iter().position(|&b| b == 0).unwrap_or(raw.len());
                if end == 0 {
                    Vec::new()
                } else {
                    let c = CString::new(&raw[..end]).map_err(|_| EINVAL)?;
                    crate::paths::resolve(&c, libc::AT_FDCWD, how)?
                        .to_bytes()
                        .to_vec()
                }
            };
            if path.len() + 1 > MAC_SUN_PATH {
                return Err(ENAMETOOLONG);
            }
            let out = addr.bytes_mut();
            out[2..2 + path.len()].copy_from_slice(&path);
            out[2 + path.len()] = 0;
            out[1] = libc::AF_UNIX as u8;
            2 + path.len() + 1
        }
        0 => {
            out[1] = libc::AF_UNSPEC as u8;
            len.min(16)
        }
        _ => return Err(EAFNOSUPPORT),
    };
    addr.bytes_mut()[0] = host_len as u8;
    addr.len = host_len as libc::socklen_t;
    Ok(addr)
}

/// Writes a host socket address of `host_len` bytes to the guest's buffer `out`,
/// in Linux's layout, and its Linux length to the `socklen_t` at `out_len`, as the
/// kernel does: truncated to the buffer, with the full length reported.
fn addr_to_guest(addr: &HostAddr, host_len: libc::socklen_t, out: u64, out_len: u64) {
    if out == 0 || out_len == 0 {
        return;
    }
    // SAFETY: the guest passes a socklen_t.
    let capacity = unsafe { std::ptr::read_unaligned(out_len as *const u32) } as usize;
    let host = addr.bytes();
    let host_len = (host_len as usize).min(host.len());
    let mut linux = [0u8; 128];
    let n = if host_len < 2 {
        0
    } else {
        match i32::from(host[1]) {
            libc::AF_INET => {
                linux[..16].copy_from_slice(&host[..16]);
                linux[..2].copy_from_slice(&2u16.to_le_bytes());
                16
            }
            libc::AF_INET6 => {
                linux[..28].copy_from_slice(&host[..28]);
                linux[..2].copy_from_slice(&LINUX_AF_INET6.to_le_bytes());
                28
            }
            libc::AF_UNIX => {
                linux[..2].copy_from_slice(&1u16.to_le_bytes());
                let raw = &host[2..host_len];
                let end = raw.iter().position(|&b| b == 0).unwrap_or(raw.len());
                let path = &raw[..end];
                if path.is_empty() {
                    2
                } else if let Some(name) = abstract_name(path) {
                    let name = &name[..name.len().min(LINUX_SUN_LEN - 3)];
                    linux[3..3 + name.len()].copy_from_slice(name);
                    3 + name.len()
                } else {
                    let path = &path[..path.len().min(LINUX_SUN_LEN - 3)];
                    linux[2..2 + path.len()].copy_from_slice(path);
                    2 + path.len() + 1
                }
            }
            family => {
                linux[..host_len].copy_from_slice(&host[..host_len]);
                linux[..2].copy_from_slice(&(family as u16).to_le_bytes());
                host_len
            }
        }
    };
    // SAFETY: the guest passes `capacity` writable bytes and a socklen_t.
    unsafe {
        std::ptr::copy_nonoverlapping(linux.as_ptr(), out as *mut u8, n.min(capacity));
        std::ptr::write_unaligned(out_len as *mut u32, n as u32);
    }
}

// --- Options ---------------------------------------------------------------------

/// How a Linux socket option is answered.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Opt {
    /// The host level and name.
    Host(libc::c_int, libc::c_int),
    /// Accepted and dropped: tuning macOS does not have, which changes nothing a
    /// program can observe.
    Ignore,
    Unsupported,
}

fn option(level: u64, name: u64) -> Opt {
    use Opt::{Host, Ignore, Unsupported};
    const IP: libc::c_int = libc::IPPROTO_IP;
    const IPV6: libc::c_int = libc::IPPROTO_IPV6;
    const TCP: libc::c_int = libc::IPPROTO_TCP;
    const SOL: libc::c_int = libc::SOL_SOCKET;
    match (level, name) {
        (1, 1) => Host(SOL, libc::SO_DEBUG),
        (1, 2) => Host(SOL, libc::SO_REUSEADDR),
        (1, 3) => Host(SOL, libc::SO_TYPE),
        (1, 4) => Host(SOL, libc::SO_ERROR),
        (1, 5) => Host(SOL, libc::SO_DONTROUTE),
        (1, 6) => Host(SOL, libc::SO_BROADCAST),
        (1, 7 | 32) => Host(SOL, libc::SO_SNDBUF), // and SO_SNDBUFFORCE
        (1, 8 | 33) => Host(SOL, libc::SO_RCVBUF), // and SO_RCVBUFFORCE
        (1, 9) => Host(SOL, libc::SO_KEEPALIVE),
        (1, 10) => Host(SOL, libc::SO_OOBINLINE),
        (1, 13) => Host(SOL, libc::SO_LINGER_SEC),
        (1, 15) => Host(SOL, libc::SO_REUSEPORT),
        (1, 18) => Host(SOL, libc::SO_RCVLOWAT),
        (1, 19) => Host(SOL, libc::SO_SNDLOWAT),
        (1, 20) => Host(SOL, libc::SO_RCVTIMEO),
        (1, 21) => Host(SOL, libc::SO_SNDTIMEO),
        (1, 29) => Host(SOL, libc::SO_TIMESTAMP),
        (1, 30) => Host(SOL, libc::SO_ACCEPTCONN),
        // SO_NO_CHECK, SO_PRIORITY, SO_PASSCRED, SO_BUSY_POLL, SO_INCOMING_CPU
        (1, 11 | 12 | 16 | 46 | 49) => Ignore,
        (0, 1) => Host(IP, libc::IP_TOS),
        (0, 2) => Host(IP, libc::IP_TTL),
        (0, 3) => Host(IP, libc::IP_HDRINCL),
        (0, 8) => Host(IP, libc::IP_PKTINFO), // IP_RECVPKTINFO on macOS,
        (0, 12) => Host(IP, libc::IP_RECVTTL),
        (0, 13) => Host(IP, 27), // IP_RECVTOS
        (0, 32) => Host(IP, libc::IP_MULTICAST_IF),
        (0, 33) => Host(IP, libc::IP_MULTICAST_TTL),
        (0, 34) => Host(IP, libc::IP_MULTICAST_LOOP),
        (0, 35) => Host(IP, libc::IP_ADD_MEMBERSHIP),
        (0, 36) => Host(IP, libc::IP_DROP_MEMBERSHIP),
        // IP_MTU_DISCOVER, IP_RECVERR, IP_FREEBIND, IP_BIND_ADDRESS_NO_PORT
        (0, 10 | 11 | 15 | 24) => Ignore,
        (41, 16) => Host(IPV6, libc::IPV6_UNICAST_HOPS),
        (41, 17) => Host(IPV6, libc::IPV6_MULTICAST_IF),
        (41, 18) => Host(IPV6, libc::IPV6_MULTICAST_HOPS),
        (41, 19) => Host(IPV6, libc::IPV6_MULTICAST_LOOP),
        (41, 20) => Host(IPV6, libc::IPV6_JOIN_GROUP),
        (41, 21) => Host(IPV6, libc::IPV6_LEAVE_GROUP),
        (41, 26) => Host(IPV6, libc::IPV6_V6ONLY),
        (41, 49) => Host(IPV6, libc::IPV6_RECVPKTINFO),
        (41, 51) => Host(IPV6, libc::IPV6_RECVHOPLIMIT),
        (41, 66) => Host(IPV6, libc::IPV6_RECVTCLASS),
        (41, 67) => Host(IPV6, libc::IPV6_TCLASS),
        // IPV6_MTU_DISCOVER, IPV6_RECVERR
        (41, 23 | 25) => Ignore,
        (6, 1) => Host(TCP, libc::TCP_NODELAY),
        (6, 2) => Host(TCP, libc::TCP_MAXSEG),
        (6, 3) => Host(TCP, libc::TCP_NOPUSH),    // TCP_CORK
        (6, 4) => Host(TCP, libc::TCP_KEEPALIVE), // TCP_KEEPIDLE
        (6, 5) => Host(TCP, libc::TCP_KEEPINTVL),
        (6, 6) => Host(TCP, libc::TCP_KEEPCNT),
        (6, 25) => Host(TCP, 0x201), // TCP_NOTSENT_LOWAT
        // TCP_DEFER_ACCEPT, TCP_QUICKACK, TCP_USER_TIMEOUT, TCP_FASTOPEN
        (6, 9 | 12 | 18 | 23) => Ignore,
        _ => Unsupported,
    }
}

const LINUX_SO_PEERCRED: u64 = 17;

/// `setsockopt(fd, level, name, value, len)`.
fn setsockopt(fd: libc::c_int, level: u64, name: u64, value: u64, len: u64) -> i64 {
    match option(level, name) {
        Opt::Host(l, n) => {
            let (mut value, mut len) = (value, len as libc::socklen_t);
            if l == libc::IPPROTO_IP {
                if n == libc::IP_ADD_MEMBERSHIP || n == libc::IP_DROP_MEMBERSHIP {
                    // Linux also takes an ip_mreqn, whose first 8 bytes are the
                    // ip_mreq macOS takes.
                    len = len.min(8);
                } else if n == libc::IP_MULTICAST_IF && len >= 8 {
                    // An ip_mreq or ip_mreqn: the interface address is its second
                    // field; macOS takes only that in_addr.
                    value += 4;
                    len = 4;
                }
            }
            // SAFETY: the guest passes `len` readable bytes of option value.
            result(i64::from(unsafe {
                libc::setsockopt(fd, l, n, value as *const libc::c_void, len)
            }))
        }
        Opt::Ignore => 0,
        Opt::Unsupported => ENOPROTOOPT,
    }
}

/// `getsockopt(fd, level, name, value, len)`.
fn getsockopt(fd: libc::c_int, level: u64, name: u64, value: u64, len: u64) -> i64 {
    if value == 0 || len == 0 {
        return EFAULT;
    }
    if level == 1 && name == LINUX_SO_PEERCRED {
        return peer_credentials(fd, value, len);
    }
    match option(level, name) {
        Opt::Host(l, n) => {
            // SAFETY: the guest passes a socklen_t and as many writable bytes.
            let status = unsafe {
                libc::getsockopt(
                    fd,
                    l,
                    n,
                    value as *mut libc::c_void,
                    len as *mut libc::socklen_t,
                )
            };
            if status != 0 {
                return result(-1);
            }
            if l == libc::SOL_SOCKET && n == libc::SO_ERROR {
                // SAFETY: SO_ERROR wrote an int.
                unsafe {
                    let e = std::ptr::read_unaligned(value as *const i32);
                    if e != 0 {
                        std::ptr::write_unaligned(value as *mut i32, -errno(e) as i32);
                    }
                }
            }
            0
        }
        Opt::Ignore => {
            // SAFETY: the guest passes a socklen_t and as many writable bytes.
            unsafe {
                if std::ptr::read_unaligned(len as *const u32) >= 4 {
                    std::ptr::write_unaligned(value as *mut i32, 0);
                    std::ptr::write_unaligned(len as *mut u32, 4);
                }
            }
            0
        }
        Opt::Unsupported => ENOPROTOOPT,
    }
}

/// `SO_PEERCRED`: Linux's `struct ucred { pid, uid, gid }` from the host's peer
/// identity calls.
fn peer_credentials(fd: libc::c_int, value: u64, len: u64) -> i64 {
    let (mut uid, mut gid) = (0, 0);
    // SAFETY: getpeereid writes two ids.
    if unsafe { libc::getpeereid(fd, &mut uid, &mut gid) } != 0 {
        return result(-1);
    }
    let mut pid: libc::pid_t = 0;
    let mut size = std::mem::size_of::<libc::pid_t>() as libc::socklen_t;
    // SAFETY: LOCAL_PEERPID writes a pid_t.
    unsafe {
        libc::getsockopt(
            fd,
            libc::SOL_LOCAL,
            libc::LOCAL_PEERPID,
            std::ptr::from_mut(&mut pid).cast(),
            &mut size,
        )
    };
    let words = [pid as u32, uid, gid];
    // SAFETY: the guest passes a socklen_t and as many writable bytes.
    unsafe {
        let n = (std::ptr::read_unaligned(len as *const u32) as usize).min(12);
        std::ptr::copy_nonoverlapping(words.as_ptr().cast::<u8>(), value as *mut u8, n);
        std::ptr::write_unaligned(len as *mut u32, 12);
    }
    0
}

// --- Messages --------------------------------------------------------------------

const MSG_ERRQUEUE: u64 = 0x2000;
const MSG_WAITFORONE: u64 = 0x10000;

/// Host `send`/`recv` flags for Linux ones. `MSG_NOSIGNAL` is every socket's
/// `SO_NOSIGPIPE`; `MSG_MORE`, `MSG_CONFIRM`, and `MSG_CMSG_CLOEXEC` only advise.
fn msg_flags(linux: u64) -> libc::c_int {
    let mut mac = (linux & 0x7) as libc::c_int; // MSG_OOB, MSG_PEEK, MSG_DONTROUTE
    for (l, m) in [
        (0x8, libc::MSG_CTRUNC),
        (0x20, libc::MSG_TRUNC),
        (0x40, libc::MSG_DONTWAIT),
        (0x80, libc::MSG_EOR),
        (0x100, libc::MSG_WAITALL),
    ] {
        if linux & l != 0 {
            mac |= m;
        }
    }
    mac
}

/// Linux `msg_flags` for host ones, after `recvmsg`.
fn linux_msg_flags(mac: libc::c_int) -> i32 {
    let mut linux = mac & 0x7;
    for (l, m) in [
        (0x8, libc::MSG_CTRUNC),
        (0x20, libc::MSG_TRUNC),
        (0x80, libc::MSG_EOR),
    ] {
        if mac & m != 0 {
            linux |= l;
        }
    }
    linux
}

/// Linux's `struct msghdr`, 56 bytes.
#[derive(Debug, Clone, Copy)]
struct LinuxMsg {
    name: u64,
    namelen: u32,
    iov: u64,
    iovlen: u64,
    control: u64,
    controllen: u64,
}

fn read_msg(p: u64) -> Option<LinuxMsg> {
    if p == 0 {
        return None;
    }
    // SAFETY: the guest passes a struct msghdr.
    unsafe {
        let word = |o: u64| std::ptr::read_unaligned((p + o) as *const u64);
        Some(LinuxMsg {
            name: word(0),
            namelen: std::ptr::read_unaligned((p + 8) as *const u32),
            iov: word(16),
            iovlen: word(24),
            control: word(32),
            controllen: word(40),
        })
    }
}

/// A control message's host level and type for Linux ones, sending.
fn cmsg_to_host(level: i32, kind: i32) -> Option<(i32, i32)> {
    Some(match (level, kind) {
        (1, 1) => (libc::SOL_SOCKET, libc::SCM_RIGHTS),
        (0, 8) => (libc::IPPROTO_IP, libc::IP_PKTINFO),
        (41, 50) => (libc::IPPROTO_IPV6, libc::IPV6_PKTINFO),
        (41, 52) => (libc::IPPROTO_IPV6, libc::IPV6_HOPLIMIT),
        (41, 67) => (libc::IPPROTO_IPV6, libc::IPV6_TCLASS),
        _ => return None,
    })
}

/// A control message's Linux level and type for host ones, receiving.
fn cmsg_to_linux(level: i32, kind: i32) -> Option<(i32, i32)> {
    Some(match (level, kind) {
        (libc::SOL_SOCKET, libc::SCM_RIGHTS) => (1, 1),
        (libc::SOL_SOCKET, libc::SCM_TIMESTAMP) => (1, 29),
        (libc::IPPROTO_IP, libc::IP_PKTINFO) => (0, 8),
        (libc::IPPROTO_IP, 27) => (0, 1), // IP_RECVTOS gives Linux's IP_TOS
        (libc::IPPROTO_IPV6, libc::IPV6_PKTINFO) => (41, 50),
        (libc::IPPROTO_IPV6, libc::IPV6_HOPLIMIT) => (41, 52),
        (libc::IPPROTO_IPV6, libc::IPV6_TCLASS) => (41, 67),
        _ => return None,
    })
}

const LINUX_CMSG_HEADER: usize = 16;
const MAC_CMSG_HEADER: usize = 12;

fn align(n: usize, to: usize) -> usize {
    n.div_ceil(to) * to
}

/// Linux control messages, in the host's layout. Those macOS has no counterpart
/// for are dropped.
fn control_to_host(linux: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut at = 0;
    while at + LINUX_CMSG_HEADER <= linux.len() {
        let field = |o: usize, n: usize| &linux[at + o..at + o + n];
        let len = u64::from_le_bytes(field(0, 8).try_into().unwrap_or([0; 8])) as usize;
        if len < LINUX_CMSG_HEADER || at + len > linux.len() {
            break;
        }
        let level = i32::from_le_bytes(field(8, 4).try_into().unwrap_or([0; 4]));
        let kind = i32::from_le_bytes(field(12, 4).try_into().unwrap_or([0; 4]));
        let data = &linux[at + LINUX_CMSG_HEADER..at + len];
        if let Some((l, k)) = cmsg_to_host(level, kind) {
            let start = out.len();
            out.extend_from_slice(&((MAC_CMSG_HEADER + data.len()) as u32).to_le_bytes());
            out.extend_from_slice(&l.to_le_bytes());
            out.extend_from_slice(&k.to_le_bytes());
            out.extend_from_slice(data);
            out.resize(start + align(MAC_CMSG_HEADER + data.len(), 4), 0);
        }
        at += align(len, 8);
    }
    out
}

/// Host control messages written to the guest's `capacity`-byte buffer at `out`
/// in Linux's layout: the length used, and whether any was dropped or cut.
fn control_to_guest(host: &[u8], out: u64, capacity: usize) -> (usize, bool) {
    let mut used = 0;
    let mut cut = false;
    let mut at = 0;
    while at + MAC_CMSG_HEADER <= host.len() {
        let word =
            |o: usize| u32::from_le_bytes(host[at + o..at + o + 4].try_into().unwrap_or([0; 4]));
        let len = word(0) as usize;
        if len < MAC_CMSG_HEADER || at + len > host.len() {
            break;
        }
        let (level, kind) = (word(4) as i32, word(8) as i32);
        let data = &host[at + MAC_CMSG_HEADER..at + len];
        at += align(len, 4);
        let Some((l, k)) = cmsg_to_linux(level, kind) else {
            cut = true;
            continue;
        };
        let linux_len = LINUX_CMSG_HEADER + data.len();
        if out == 0 || used + linux_len > capacity {
            cut = true;
            if (level, kind) == (libc::SOL_SOCKET, libc::SCM_RIGHTS) {
                // Descriptors that do not reach the guest are closed, as Linux
                // does with the ones a short buffer cannot hold.
                for fd in data.chunks_exact(4) {
                    let fd = i32::from_le_bytes(fd.try_into().unwrap_or([0; 4]));
                    // SAFETY: the descriptor arrived with this message.
                    unsafe { libc::close(fd) };
                }
            }
            continue;
        }
        let mut header = [0u8; LINUX_CMSG_HEADER];
        header[..8].copy_from_slice(&(linux_len as u64).to_le_bytes());
        header[8..12].copy_from_slice(&l.to_le_bytes());
        header[12..16].copy_from_slice(&k.to_le_bytes());
        // SAFETY: `used + linux_len` is within the guest's `capacity` bytes.
        unsafe {
            let dst = (out as usize + used) as *mut u8;
            std::ptr::copy_nonoverlapping(header.as_ptr(), dst, LINUX_CMSG_HEADER);
            std::ptr::copy_nonoverlapping(data.as_ptr(), dst.add(LINUX_CMSG_HEADER), data.len());
        }
        used = (used + align(linux_len, 8)).min(capacity);
    }
    (used, cut)
}

/// `sendmsg(fd, msg, flags)` with a Linux `msghdr`.
fn sendmsg(fd: libc::c_int, msg: u64, flags: u64) -> i64 {
    let Some(m) = read_msg(msg) else {
        return EFAULT;
    };
    let addr = if m.name != 0 && m.namelen != 0 {
        match addr_from_guest(m.name, u64::from(m.namelen), Use::Read) {
            Ok(a) => Some(a),
            Err(e) => return e,
        }
    } else {
        None
    };
    let control = if m.control != 0 && m.controllen != 0 {
        // SAFETY: the guest passes `controllen` readable bytes.
        let linux =
            unsafe { std::slice::from_raw_parts(m.control as *const u8, m.controllen as usize) };
        control_to_host(linux)
    } else {
        Vec::new()
    };
    // macOS reads control messages as 4-byte aligned words.
    let mut aligned = vec![0u32; control.len().div_ceil(4)];
    // SAFETY: `aligned` has at least `control.len()` bytes.
    unsafe {
        std::ptr::copy_nonoverlapping(
            control.as_ptr(),
            aligned.as_mut_ptr().cast::<u8>(),
            control.len(),
        );
    }
    // SAFETY: msghdr is plain data.
    let mut h: libc::msghdr = unsafe { std::mem::zeroed() };
    if let Some(a) = &addr {
        h.msg_name = a.as_ptr() as *mut libc::c_void;
        h.msg_namelen = a.len;
    }
    h.msg_iov = m.iov as *mut libc::iovec;
    h.msg_iovlen = m.iovlen as libc::c_int;
    if !control.is_empty() {
        h.msg_control = aligned.as_mut_ptr().cast();
        h.msg_controllen = control.len() as libc::socklen_t;
    }
    // SAFETY: the header points at the guest's iovecs and this frame's buffers.
    result(unsafe { libc::sendmsg(fd, &h, msg_flags(flags)) } as i64)
}

/// `recvmsg(fd, msg, flags)` with a Linux `msghdr`, updated as the kernel does.
fn recvmsg(fd: libc::c_int, msg: u64, flags: u64) -> i64 {
    if flags & MSG_ERRQUEUE != 0 {
        // No error queue: nothing is ever waiting in it.
        return EAGAIN;
    }
    let Some(m) = read_msg(msg) else {
        return EFAULT;
    };
    let mut addr = HostAddr::empty();
    // Host control messages are smaller than Linux ones; this always holds what
    // the guest's buffer could.
    let mut control = vec![0u32; (m.controllen as usize).div_ceil(4) + 16];
    // SAFETY: msghdr is plain data.
    let mut h: libc::msghdr = unsafe { std::mem::zeroed() };
    if m.name != 0 {
        h.msg_name = addr.as_mut_ptr().cast();
        h.msg_namelen = addr.len;
    }
    h.msg_iov = m.iov as *mut libc::iovec;
    h.msg_iovlen = m.iovlen as libc::c_int;
    if m.control != 0 && m.controllen != 0 {
        h.msg_control = control.as_mut_ptr().cast();
        h.msg_controllen = (control.len() * 4) as libc::socklen_t;
    }
    // SAFETY: the header points at the guest's iovecs and this frame's buffers.
    let n = unsafe { libc::recvmsg(fd, &mut h, msg_flags(flags)) };
    if n < 0 {
        return result(-1);
    }
    if m.name != 0 {
        addr_to_guest(&addr, h.msg_namelen, m.name, msg + 8);
    }
    let host_control = if h.msg_control.is_null() {
        &[][..]
    } else {
        // SAFETY: recvmsg wrote `msg_controllen` bytes of the buffer.
        unsafe {
            std::slice::from_raw_parts(
                control.as_ptr().cast::<u8>(),
                (h.msg_controllen as usize).min(control.len() * 4),
            )
        }
    };
    let (used, cut) = control_to_guest(host_control, m.control, m.controllen as usize);
    let mut out_flags = linux_msg_flags(h.msg_flags);
    if cut {
        out_flags |= 0x8; // MSG_CTRUNC
    }
    // SAFETY: the guest's msghdr has msg_controllen at 40 and msg_flags at 48.
    unsafe {
        std::ptr::write_unaligned((msg + 40) as *mut u64, used as u64);
        std::ptr::write_unaligned((msg + 48) as *mut i32, out_flags);
    }
    n as i64
}

/// `recvmmsg` and `sendmmsg`: one call per Linux `mmsghdr` (a `msghdr` and a
/// 32-bit length, 64 bytes), as the kernel does them in a loop.
fn each_message(
    fd: libc::c_int,
    vec: u64,
    count: u64,
    flags: u64,
    one: fn(libc::c_int, u64, u64) -> i64,
) -> i64 {
    let mut flags = flags;
    let mut done = 0;
    for i in 0..count.min(1024) {
        let entry = vec + 64 * i;
        let n = one(fd, entry, flags & !MSG_WAITFORONE);
        if n < 0 {
            return if done == 0 { n } else { done };
        }
        // SAFETY: the guest's mmsghdr has msg_len at 56.
        unsafe { std::ptr::write_unaligned((entry + 56) as *mut u32, n as u32) };
        done += 1;
        if flags & MSG_WAITFORONE != 0 {
            flags |= 0x40; // MSG_DONTWAIT after the first
        }
    }
    done
}

// --- Calls -----------------------------------------------------------------------

fn socket(domain: u64, kind: u64, protocol: u64) -> i64 {
    let Some(domain) = host_domain(domain) else {
        return EAFNOSUPPORT;
    };
    // SAFETY: a plain socket call.
    let fd = unsafe { libc::socket(domain, (kind & 0xf) as libc::c_int, protocol as libc::c_int) };
    if fd < 0 {
        return result(-1);
    }
    prepare(fd, kind);
    i64::from(fd)
}

fn socketpair(domain: u64, kind: u64, protocol: u64, out: u64) -> i64 {
    if out == 0 {
        return EFAULT;
    }
    let Some(domain) = host_domain(domain) else {
        return EAFNOSUPPORT;
    };
    let mut fds = [0 as libc::c_int; 2];
    // SAFETY: socketpair writes two descriptors into the array.
    let status = unsafe {
        libc::socketpair(
            domain,
            (kind & 0xf) as libc::c_int,
            protocol as libc::c_int,
            fds.as_mut_ptr(),
        )
    };
    if status != 0 {
        return result(-1);
    }
    for fd in fds {
        prepare(fd, kind);
    }
    // SAFETY: the guest passes an int[2].
    unsafe { std::ptr::write_unaligned(out as *mut [libc::c_int; 2], fds) };
    0
}

fn bind(fd: libc::c_int, addr: u64, len: u64) -> i64 {
    let a = match addr_from_guest(addr, len, Use::Replace) {
        Ok(a) => a,
        Err(e) => return e,
    };
    if a.stands_for_abstract {
        // An abstract name has no file to outlive its socket; one left behind by
        // an earlier process stands for nothing.
        let path = &a.bytes()[2..a.len as usize - 1];
        if let Ok(c) = CString::new(path) {
            // SAFETY: unlink of a path in cs2metal's own directory.
            unsafe { libc::unlink(c.as_ptr()) };
        }
    }
    // SAFETY: the address is a host sockaddr of `len` bytes.
    result(i64::from(unsafe { libc::bind(fd, a.as_ptr(), a.len) }))
}

fn connect(fd: libc::c_int, addr: u64, len: u64) -> i64 {
    match addr_from_guest(addr, len, Use::Read) {
        // SAFETY: the address is a host sockaddr of `len` bytes.
        Ok(a) => result(i64::from(unsafe { libc::connect(fd, a.as_ptr(), a.len) })),
        Err(e) => e,
    }
}

fn accept(fd: libc::c_int, addr: u64, len: u64, flags: u64) -> i64 {
    let mut a = HostAddr::empty();
    let mut host_len = a.len;
    // SAFETY: accept writes at most `host_len` bytes of address.
    let new = unsafe { libc::accept(fd, a.as_mut_ptr(), &mut host_len) };
    if new < 0 {
        return result(-1);
    }
    prepare(new, flags);
    addr_to_guest(&a, host_len, addr, len);
    i64::from(new)
}

type NameCall =
    unsafe extern "C" fn(libc::c_int, *mut libc::sockaddr, *mut libc::socklen_t) -> libc::c_int;

fn name(fd: libc::c_int, addr: u64, len: u64, call: NameCall) -> i64 {
    if addr == 0 || len == 0 {
        return EFAULT;
    }
    let mut a = HostAddr::empty();
    let mut host_len = a.len;
    // SAFETY: the call writes at most `host_len` bytes of address.
    if unsafe { call(fd, a.as_mut_ptr(), &mut host_len) } != 0 {
        return result(-1);
    }
    addr_to_guest(&a, host_len, addr, len);
    0
}

fn sendto(fd: libc::c_int, buf: u64, len: u64, flags: u64, addr: u64, alen: u64) -> i64 {
    let to = if addr != 0 {
        match addr_from_guest(addr, alen, Use::Read) {
            Ok(a) => Some(a),
            Err(e) => return e,
        }
    } else {
        None
    };
    let (ptr, plen) = to
        .as_ref()
        .map_or((std::ptr::null(), 0), |a| (a.as_ptr(), a.len));
    // SAFETY: the guest passes `len` readable bytes; the address is the host's.
    result(unsafe {
        libc::sendto(
            fd,
            buf as *const libc::c_void,
            len as usize,
            msg_flags(flags),
            ptr,
            plen,
        )
    } as i64)
}

fn recvfrom(fd: libc::c_int, buf: u64, len: u64, flags: u64, addr: u64, alen: u64) -> i64 {
    if flags & MSG_ERRQUEUE != 0 {
        return EAGAIN;
    }
    let mut a = HostAddr::empty();
    let mut host_len = a.len;
    let (ptr, lenp) = if addr != 0 {
        (a.as_mut_ptr(), std::ptr::from_mut(&mut host_len))
    } else {
        (std::ptr::null_mut(), std::ptr::null_mut())
    };
    // SAFETY: the guest passes `len` writable bytes; the address is the host's.
    let n = unsafe {
        libc::recvfrom(
            fd,
            buf as *mut libc::c_void,
            len as usize,
            msg_flags(flags),
            ptr,
            lenp,
        )
    };
    if n < 0 {
        return result(-1);
    }
    if addr != 0 {
        addr_to_guest(&a, host_len, addr, alen);
    }
    n as i64
}

/// `ioctl` requests on sockets and pipes: `FIONREAD` and `FIONBIO`.
fn ioctl(fd: libc::c_int, request: u64, arg: u64) -> Option<i64> {
    match request {
        0x541b => {
            // SAFETY: FIONREAD writes an int to the guest's pointer.
            Some(result(i64::from(unsafe {
                libc::ioctl(fd, libc::FIONREAD, arg as *mut libc::c_int)
            })))
        }
        0x5421 => {
            if arg == 0 {
                return Some(EFAULT);
            }
            // SAFETY: the guest passes an int.
            let on = unsafe { std::ptr::read_unaligned(arg as *const libc::c_int) } != 0;
            // SAFETY: fcntl on the guest's descriptor.
            let status = unsafe {
                let flags = libc::fcntl(fd, libc::F_GETFL);
                let flags = if on {
                    flags | libc::O_NONBLOCK
                } else {
                    flags & !libc::O_NONBLOCK
                };
                libc::fcntl(fd, libc::F_SETFL, flags)
            };
            Some(result(i64::from(status)))
        }
        _ => None,
    }
}

/// Handles the system calls of this module.
pub(crate) fn answer(f: &SyscallFrame) -> Option<i64> {
    let (a0, a1, a2, a3, a4, a5) = (f.arg0, f.arg1, f.arg2, f.arg3, f.arg4, f.arg5);
    let fd = a0 as libc::c_int;
    Some(match f.number {
        16 => return ioctl(fd, a1, a2),
        41 => socket(a0, a1, a2),
        42 => connect(fd, a1, a2),
        43 => accept(fd, a1, a2, 0),
        44 => sendto(fd, a1, a2, a3, a4, a5),
        45 => recvfrom(fd, a1, a2, a3, a4, a5),
        46 => sendmsg(fd, a1, a2),
        47 => recvmsg(fd, a1, a2),
        // SAFETY: plain calls on the guest's descriptor.
        48 => result(i64::from(unsafe { libc::shutdown(fd, a1 as libc::c_int) })),
        49 => bind(fd, a1, a2),
        // SAFETY: as above.
        50 => result(i64::from(unsafe { libc::listen(fd, a1 as libc::c_int) })),
        51 => name(fd, a1, a2, libc::getsockname),
        52 => name(fd, a1, a2, libc::getpeername),
        53 => socketpair(a0, a1, a2, a3),
        54 => setsockopt(fd, a1, a2, a3, a4),
        55 => getsockopt(fd, a1, a2, a3, a4),
        288 => accept(fd, a1, a2, a3),
        299 => each_message(fd, a1, a2, a3, recvmsg),
        307 => each_message(fd, a1, a2, a3, sendmsg),
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn linux_inet(port: u16, ip: [u8; 4]) -> [u8; 16] {
        let mut a = [0u8; 16];
        a[..2].copy_from_slice(&2u16.to_le_bytes());
        a[2..4].copy_from_slice(&port.to_be_bytes());
        a[4..8].copy_from_slice(&ip);
        a
    }

    #[test]
    fn options_map_to_the_host_or_are_answered() {
        assert_eq!(
            option(1, 2),
            Opt::Host(libc::SOL_SOCKET, libc::SO_REUSEADDR)
        );
        assert_eq!(
            option(41, 26),
            Opt::Host(libc::IPPROTO_IPV6, libc::IPV6_V6ONLY)
        );
        assert_eq!(option(0, 10), Opt::Ignore, "IP_MTU_DISCOVER");
        assert_eq!(option(17, 103), Opt::Unsupported, "UDP_SEGMENT");
        assert_eq!(
            msg_flags(0x40 | 0x4000),
            libc::MSG_DONTWAIT,
            "MSG_NOSIGNAL drops"
        );
        assert_eq!(linux_msg_flags(libc::MSG_TRUNC), 0x20);
    }

    #[test]
    fn a_udp_socket_binds_sends_and_receives_in_linux_layout() {
        let fd = socket(2, 2 | SOCK_NONBLOCK, 0);
        assert!(fd >= 0);
        let fd = fd as libc::c_int;
        let local = linux_inet(0, [127, 0, 0, 1]);
        assert_eq!(bind(fd, local.as_ptr() as u64, 16), 0);
        let mut name_buf = [0u8; 16];
        let mut name_len: u32 = 16;
        let got = name(
            fd,
            name_buf.as_mut_ptr() as u64,
            std::ptr::from_mut(&mut name_len) as u64,
            libc::getsockname,
        );
        assert_eq!(got, 0);
        assert_eq!(name_len, 16);
        assert_eq!(&name_buf[..2], &2u16.to_le_bytes(), "Linux family field");
        assert_eq!(&name_buf[4..8], &[127, 0, 0, 1]);
        assert_ne!(&name_buf[2..4], &[0, 0], "a port was chosen");

        let message = b"datagram";
        let sent = sendto(
            fd,
            message.as_ptr() as u64,
            message.len() as u64,
            0x4000, // MSG_NOSIGNAL
            name_buf.as_ptr() as u64,
            16,
        );
        assert_eq!(sent, message.len() as i64);
        let mut buf = [0u8; 32];
        let mut from = [0u8; 16];
        let mut from_len: u32 = 16;
        let mut got = EAGAIN;
        for _ in 0..100 {
            got = recvfrom(
                fd,
                buf.as_mut_ptr() as u64,
                32,
                0,
                from.as_mut_ptr() as u64,
                std::ptr::from_mut(&mut from_len) as u64,
            );
            if got != EAGAIN {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert_eq!(got, message.len() as i64);
        assert_eq!(&buf[..message.len()], message);
        assert_eq!(from, name_buf, "the sender's address, in Linux layout");
        // SAFETY: the test's socket.
        unsafe { libc::close(fd) };
    }

    #[test]
    fn a_socketpair_passes_a_descriptor_in_linux_control_messages() {
        let mut pair = [0 as libc::c_int; 2];
        assert_eq!(
            socketpair(1, 1 | SOCK_CLOEXEC, 0, pair.as_mut_ptr() as u64),
            0
        );
        // SAFETY: fcntl on the test's descriptor.
        let fd_flags = unsafe { libc::fcntl(pair[0], libc::F_GETFD) };
        assert_eq!(fd_flags, libc::FD_CLOEXEC);

        let mut pipe = [0 as libc::c_int; 2];
        // SAFETY: pipe writes two descriptors.
        assert_eq!(unsafe { libc::pipe(pipe.as_mut_ptr()) }, 0);
        let payload = *b"x";
        let iov = libc::iovec {
            iov_base: payload.as_ptr() as *mut libc::c_void,
            iov_len: 1,
        };
        // Linux cmsghdr: u64 len, i32 level, i32 type, then the int, padded to 24.
        let mut control = [0u8; 24];
        control[..8].copy_from_slice(&20u64.to_le_bytes());
        control[8..12].copy_from_slice(&1i32.to_le_bytes());
        control[12..16].copy_from_slice(&1i32.to_le_bytes());
        control[16..20].copy_from_slice(&pipe[1].to_le_bytes());
        let mut msg = [0u64; 7];
        msg[2] = std::ptr::from_ref(&iov) as u64;
        msg[3] = 1;
        msg[4] = control.as_ptr() as u64;
        msg[5] = 24;
        assert_eq!(sendmsg(pair[0], msg.as_ptr() as u64, 0x4000), 1);

        let mut back = [0u8; 1];
        let iov = libc::iovec {
            iov_base: back.as_mut_ptr().cast(),
            iov_len: 1,
        };
        let mut received = [0u8; 64];
        let mut msg = [0u64; 7];
        msg[2] = std::ptr::from_ref(&iov) as u64;
        msg[3] = 1;
        msg[4] = received.as_mut_ptr() as u64;
        msg[5] = 64;
        assert_eq!(recvmsg(pair[1], msg.as_mut_ptr() as u64, 0), 1);
        assert_eq!(back, payload);
        assert_eq!(msg[5], 24, "one Linux SCM_RIGHTS message");
        assert_eq!(
            u64::from_le_bytes(received[..8].try_into().expect("bytes")),
            20
        );
        assert_eq!(&received[8..16], &[1, 0, 0, 0, 1, 0, 0, 0]);
        let fd = i32::from_le_bytes(received[16..20].try_into().expect("bytes"));
        // SAFETY: write to the descriptor just received, then read the pipe.
        unsafe {
            assert_eq!(libc::write(fd, b"y".as_ptr().cast(), 1), 1);
            let mut b = [0u8; 1];
            assert_eq!(libc::read(pipe[0], b.as_mut_ptr().cast(), 1), 1);
            assert_eq!(&b, b"y");
            for f in [pair[0], pair[1], pipe[0], pipe[1], fd] {
                libc::close(f);
            }
        }
    }

    #[test]
    fn an_abstract_unix_socket_keeps_its_linux_name() {
        let mut linux = [0u8; 2 + 1 + 12];
        linux[0] = 1;
        let name = b"cs2metal-ut-";
        linux[3..].copy_from_slice(name);
        let server = socket(1, 1, 0) as libc::c_int;
        assert_eq!(bind(server, linux.as_ptr() as u64, linux.len() as u64), 0);
        // SAFETY: listen on the test's socket.
        assert_eq!(unsafe { libc::listen(server, 1) }, 0);
        let mut out = [0u8; 110];
        let mut out_len: u32 = 110;
        name_of(server, &mut out, &mut out_len);
        assert_eq!(out_len as usize, linux.len());
        assert_eq!(&out[..linux.len()], &linux[..]);

        let client = socket(1, 1, 0) as libc::c_int;
        assert_eq!(
            connect(client, linux.as_ptr() as u64, linux.len() as u64),
            0
        );
        // SAFETY: the test's sockets.
        unsafe {
            libc::close(client);
            libc::close(server);
        }
    }

    fn name_of(fd: libc::c_int, out: &mut [u8], len: &mut u32) {
        let got = name(
            fd,
            out.as_mut_ptr() as u64,
            std::ptr::from_mut(len) as u64,
            libc::getsockname,
        );
        assert_eq!(got, 0);
    }

    #[test]
    fn control_messages_convert_both_ways() {
        let mut linux = [0u8; 24];
        linux[..8].copy_from_slice(&20u64.to_le_bytes());
        linux[8..12].copy_from_slice(&1i32.to_le_bytes());
        linux[12..16].copy_from_slice(&1i32.to_le_bytes());
        linux[16..20].copy_from_slice(&7i32.to_le_bytes());
        let host = control_to_host(&linux);
        assert_eq!(host.len(), 16);
        assert_eq!(u32::from_le_bytes(host[..4].try_into().expect("bytes")), 16);
        assert_eq!(
            i32::from_le_bytes(host[4..8].try_into().expect("bytes")),
            libc::SOL_SOCKET
        );
        let mut back = [0u8; 24];
        let (used, cut) = control_to_guest(&host, back.as_mut_ptr() as u64, 24);
        assert_eq!((used, cut), (24, false));
        assert_eq!(&back[..20], &linux[..20]);
    }
}
