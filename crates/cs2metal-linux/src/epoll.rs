//! `epoll` on top of `kqueue`.
//!
//! An epoll descriptor is a kqueue descriptor. Each registered descriptor gets a
//! read filter, a write filter, or both, per its event mask; `EPOLLET` becomes
//! `EV_CLEAR` and `EPOLLONESHOT` `EV_ONESHOT`. `epoll_wait` gathers the kqueue
//! events of each descriptor into one Linux `struct epoll_event`, which on x86-64
//! is packed: a 32-bit mask, then the 64-bit user data, 12 bytes in all.

use std::collections::HashMap;
use std::sync::Mutex;

use crate::translate::{EBADF, EFAULT, EINVAL, ENOENT, result};

const EPOLLIN: u32 = 0x1;
const EPOLLPRI: u32 = 0x2;
const EPOLLOUT: u32 = 0x4;
const EPOLLERR: u32 = 0x8;
const EPOLLHUP: u32 = 0x10;
const EPOLLRDHUP: u32 = 0x2000;
const EPOLLONESHOT: u32 = 1 << 30;
const EPOLLET: u32 = 1 << 31;

const CTL_ADD: u64 = 1;
const CTL_DEL: u64 = 2;
const CTL_MOD: u64 = 3;

/// Registrations per epoll descriptor: descriptor to (event mask, user data).
type Interest = HashMap<i32, (u32, u64)>;

static EPOLLS: Mutex<Option<HashMap<i32, Interest>>> = Mutex::new(None);

/// `epoll_create1(flags)`; `epoll_create(size)` too.
pub(crate) fn create(flags: u64) -> i64 {
    // SAFETY: kqueue has no preconditions.
    let kq = unsafe { libc::kqueue() };
    if kq < 0 {
        return result(-1);
    }
    if flags & 0o2000000 != 0 {
        // SAFETY: fcntl on the descriptor just made.
        unsafe { libc::fcntl(kq, libc::F_SETFD, libc::FD_CLOEXEC) };
    }
    if let Ok(mut all) = EPOLLS.lock() {
        all.get_or_insert_with(HashMap::new)
            .insert(kq, Interest::new());
    }
    i64::from(kq)
}

fn change(fd: i32, filter: i16, flags: u16) -> libc::kevent {
    libc::kevent {
        ident: fd as usize,
        filter,
        flags,
        fflags: 0,
        data: 0,
        udata: std::ptr::null_mut(),
    }
}

/// Applies one descriptor's mask to the kqueue: filters it wants are added (or
/// re-enabled), filters it no longer wants are deleted.
fn apply(kq: i32, fd: i32, mask: Option<u32>) -> i64 {
    let mut changes = Vec::new();
    for (filter, wanted) in [
        (libc::EVFILT_READ, EPOLLIN | EPOLLPRI | EPOLLRDHUP),
        (libc::EVFILT_WRITE, EPOLLOUT),
    ] {
        match mask {
            Some(m) if m & wanted != 0 => {
                let mut flags = libc::EV_ADD | libc::EV_ENABLE;
                if m & EPOLLET != 0 {
                    flags |= libc::EV_CLEAR;
                }
                if m & EPOLLONESHOT != 0 {
                    flags |= libc::EV_ONESHOT;
                }
                changes.push(change(fd, filter, flags));
            }
            _ => changes.push(change(fd, filter, libc::EV_DELETE)),
        }
    }
    for c in &changes {
        // SAFETY: one change, no events out. Deleting a filter that was never added
        // fails with ENOENT, which is expected here and ignored.
        let status = unsafe { libc::kevent(kq, c, 1, std::ptr::null_mut(), 0, std::ptr::null()) };
        if status < 0 && c.flags & libc::EV_DELETE == 0 {
            return result(-1);
        }
    }
    0
}

/// `epoll_ctl(epfd, op, fd, event)`.
pub(crate) fn ctl(epfd: i64, op: u64, fd: i64, event: u64) -> i64 {
    let (epfd, fd) = (epfd as i32, fd as i32);
    let Ok(mut all) = EPOLLS.lock() else {
        return EINVAL;
    };
    let Some(interest) = all.as_mut().and_then(|a| a.get_mut(&epfd)) else {
        return EBADF;
    };
    let read_event = || -> Option<(u32, u64)> {
        (event != 0).then(|| {
            // SAFETY: the guest passes a packed struct epoll_event of 12 bytes.
            unsafe {
                (
                    std::ptr::read_unaligned(event as *const u32),
                    std::ptr::read_unaligned((event + 4) as *const u64),
                )
            }
        })
    };
    match op {
        CTL_ADD | CTL_MOD => {
            let Some((mask, data)) = read_event() else {
                return EFAULT;
            };
            if op == CTL_MOD && !interest.contains_key(&fd) {
                return ENOENT;
            }
            let status = apply(epfd, fd, Some(mask));
            if status == 0 {
                interest.insert(fd, (mask, data));
            }
            status
        }
        CTL_DEL => {
            if interest.remove(&fd).is_none() {
                return ENOENT;
            }
            apply(epfd, fd, None)
        }
        _ => EINVAL,
    }
}

/// `epoll_wait(epfd, events, maxevents, timeout)`, timeout in milliseconds.
pub(crate) fn wait(epfd: i64, events: u64, max: u64, timeout: i64) -> i64 {
    let epfd = epfd as i32;
    if max == 0 || events == 0 {
        return EINVAL;
    }
    let max = max.min(1024) as usize;
    let mut out: Vec<libc::kevent> = vec![change(0, 0, 0); max * 2];
    let ts = libc::timespec {
        tv_sec: (timeout / 1000) as libc::time_t,
        tv_nsec: ((timeout % 1000) * 1_000_000) as libc::c_long,
    };
    let wait = if timeout < 0 { std::ptr::null() } else { &ts };
    // SAFETY: out has room for the requested events; no changes are passed.
    let n = unsafe {
        libc::kevent(
            epfd,
            std::ptr::null(),
            0,
            out.as_mut_ptr(),
            out.len() as i32,
            wait,
        )
    };
    if n < 0 {
        return result(-1);
    }
    let Ok(all) = EPOLLS.lock() else {
        return EINVAL;
    };
    let Some(interest) = all.as_ref().and_then(|a| a.get(&epfd)) else {
        return EBADF;
    };
    // One Linux event per descriptor, in the order kqueue reported them.
    let mut merged: Vec<(i32, u32)> = Vec::new();
    for e in &out[..n as usize] {
        let fd = e.ident as i32;
        let mut mask = match e.filter {
            libc::EVFILT_READ => EPOLLIN,
            libc::EVFILT_WRITE => EPOLLOUT,
            _ => 0,
        };
        if e.flags & libc::EV_EOF != 0 {
            mask |= EPOLLHUP
                | if e.filter == libc::EVFILT_READ {
                    EPOLLRDHUP
                } else {
                    0
                };
        }
        if e.flags & libc::EV_ERROR != 0 {
            mask |= EPOLLERR;
        }
        match merged.iter_mut().find(|(f, _)| *f == fd) {
            Some((_, m)) => *m |= mask,
            None => merged.push((fd, mask)),
        }
    }
    let mut written = 0usize;
    for (fd, mask) in merged.into_iter().take(max) {
        let Some(&(wanted, data)) = interest.get(&fd) else {
            continue;
        };
        let mask = mask & (wanted | EPOLLERR | EPOLLHUP);
        let at = events + 12 * written as u64;
        // SAFETY: the guest passes room for maxevents packed 12-byte structs.
        unsafe {
            std::ptr::write_unaligned(at as *mut u32, mask);
            std::ptr::write_unaligned((at + 4) as *mut u64, data);
        }
        written += 1;
    }
    written as i64
}

/// Forgets an epoll descriptor being closed.
pub(crate) fn close(fd: i64) {
    if let Ok(mut all) = EPOLLS.lock() {
        if let Some(a) = all.as_mut() {
            a.remove(&(fd as i32));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn epoll_reports_a_readable_pipe_with_its_user_data() {
        let ep = create(0);
        assert!(ep >= 0);
        let mut fds = [0i32; 2];
        // SAFETY: pipe writes two descriptors.
        assert_eq!(unsafe { libc::pipe(fds.as_mut_ptr()) }, 0);
        let mut event = [0u8; 12];
        event[..4].copy_from_slice(&EPOLLIN.to_le_bytes());
        event[4..].copy_from_slice(&0xfeed_u64.to_le_bytes());
        assert_eq!(
            ctl(ep, CTL_ADD, i64::from(fds[0]), event.as_ptr() as u64),
            0
        );

        let mut out = [0u8; 24];
        assert_eq!(wait(ep, out.as_mut_ptr() as u64, 2, 0), 0, "nothing yet");
        // SAFETY: one byte into the pipe.
        unsafe { libc::write(fds[1], [7u8].as_ptr().cast(), 1) };
        assert_eq!(wait(ep, out.as_mut_ptr() as u64, 2, 100), 1);
        assert_eq!(u32::from_le_bytes(out[..4].try_into().expect("4")), EPOLLIN);
        assert_eq!(
            u64::from_le_bytes(out[4..12].try_into().expect("8")),
            0xfeed
        );

        assert_eq!(ctl(ep, CTL_DEL, i64::from(fds[0]), 0), 0);
        assert_eq!(wait(ep, out.as_mut_ptr() as u64, 2, 0), 0, "deleted");
        assert_eq!(
            ctl(ep, CTL_MOD, i64::from(fds[0]), event.as_ptr() as u64),
            ENOENT
        );
        close(ep);
        // SAFETY: the test's own descriptors.
        unsafe {
            libc::close(fds[0]);
            libc::close(fds[1]);
            libc::close(ep as i32);
        }
    }
}
