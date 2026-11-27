//! File descriptors whose Linux semantics macOS lacks: `eventfd`, and the
//! directory streams behind `getdents64`.
//!
//! An eventfd is the read end of a host pipe plus a 64-bit counter. The pipe holds
//! one byte while the counter is non-zero, so `poll` and `epoll` see the descriptor
//! readable exactly when a Linux eventfd would be.

use std::collections::HashMap;
use std::sync::{Condvar, Mutex};

use crate::translate::{EAGAIN, EFAULT, EINVAL, result};

struct EventFd {
    /// The write end of the pipe whose read end is the guest's descriptor.
    signal: libc::c_int,
    counter: u64,
    semaphore: bool,
    nonblocking: bool,
}

static EVENTS: Mutex<Option<HashMap<libc::c_int, EventFd>>> = Mutex::new(None);
static CHANGED: Condvar = Condvar::new();

const EFD_SEMAPHORE: u64 = 1;
const EFD_NONBLOCK: u64 = 0o4000;
const EFD_CLOEXEC: u64 = 0o2000000;

fn set_flags(fd: libc::c_int, cloexec: bool, nonblocking: bool) {
    // SAFETY: fcntl on descriptors this module just created.
    unsafe {
        if cloexec {
            libc::fcntl(fd, libc::F_SETFD, libc::FD_CLOEXEC);
        }
        if nonblocking {
            let flags = libc::fcntl(fd, libc::F_GETFL);
            libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK);
        }
    }
}

/// `eventfd2(initval, flags)`.
pub(crate) fn eventfd(initial: u64, flags: u64) -> i64 {
    let mut pipe = [0 as libc::c_int; 2];
    // SAFETY: pipe writes two descriptors into the array.
    if unsafe { libc::pipe(pipe.as_mut_ptr()) } != 0 {
        return result(-1);
    }
    let [read, write] = pipe;
    set_flags(read, flags & EFD_CLOEXEC != 0, true);
    set_flags(write, true, true);
    if initial != 0 {
        // SAFETY: one byte into the pipe just made.
        unsafe { libc::write(write, [1u8].as_ptr().cast(), 1) };
    }
    let event = EventFd {
        signal: write,
        counter: initial,
        semaphore: flags & EFD_SEMAPHORE != 0,
        nonblocking: flags & EFD_NONBLOCK != 0,
    };
    if let Ok(mut events) = EVENTS.lock() {
        events.get_or_insert_with(HashMap::new).insert(read, event);
    }
    i64::from(read)
}

fn drain(fd: libc::c_int) {
    let mut byte = [0u8; 16];
    // SAFETY: the descriptor is the pipe's non-blocking read end.
    while unsafe { libc::read(fd, byte.as_mut_ptr().cast(), byte.len()) } > 0 {}
}

/// `read` on an eventfd, or `None` if `fd` is not one.
pub(crate) fn read(fd: libc::c_int, buf: u64, len: u64) -> Option<i64> {
    let mut events = EVENTS.lock().ok()?;
    events.as_ref()?.get(&fd)?;
    if len < 8 {
        return Some(EINVAL);
    }
    if buf == 0 {
        return Some(EFAULT);
    }
    loop {
        let event = events.as_mut()?.get_mut(&fd)?;
        if event.counter > 0 {
            let value = if event.semaphore { 1 } else { event.counter };
            event.counter -= value;
            if event.counter == 0 {
                drain(fd);
            }
            // SAFETY: the guest passed at least 8 bytes.
            unsafe { std::ptr::write_unaligned(buf as *mut u64, value) };
            return Some(8);
        }
        if event.nonblocking {
            return Some(EAGAIN);
        }
        events = CHANGED.wait(events).ok()?;
    }
}

/// `write` on an eventfd, or `None` if `fd` is not one.
pub(crate) fn write(fd: libc::c_int, buf: u64, len: u64) -> Option<i64> {
    let mut events = EVENTS.lock().ok()?;
    let event = events.as_mut()?.get_mut(&fd)?;
    if len < 8 || buf == 0 {
        return Some(EINVAL);
    }
    // SAFETY: the guest passed at least 8 bytes.
    let value = unsafe { std::ptr::read_unaligned(buf as *const u64) };
    if value == u64::MAX {
        return Some(EINVAL);
    }
    let was_zero = event.counter == 0;
    event.counter = event.counter.saturating_add(value).min(u64::MAX - 1);
    if was_zero && event.counter > 0 {
        // SAFETY: one byte into the eventfd's own pipe.
        unsafe { libc::write(event.signal, [1u8].as_ptr().cast(), 1) };
    }
    CHANGED.notify_all();
    Some(8)
}

/// Forgets an eventfd being closed, closing its pipe's other end.
pub(crate) fn close(fd: libc::c_int) {
    if let Ok(mut events) = EVENTS.lock() {
        if let Some(event) = events.as_mut().and_then(|e| e.remove(&fd)) {
            // SAFETY: the write end belongs to this eventfd only.
            unsafe { libc::close(event.signal) };
        }
    }
}

/// An open directory stream per descriptor, for `getdents64`.
struct Dir(*mut libc::DIR);

// SAFETY: a DIR is used only under the DIRS lock.
unsafe impl Send for Dir {}

static DIRS: Mutex<Option<HashMap<libc::c_int, Dir>>> = Mutex::new(None);

/// Linux `d_type` from macOS's: the values are the same (DT_REG 8, DT_DIR 4, ...).
fn linux_dirent(out: &mut Vec<u8>, ino: u64, offset: i64, kind: u8, name: &[u8]) {
    let reclen = (19 + name.len() + 1).next_multiple_of(8);
    out.extend(ino.to_le_bytes());
    out.extend(offset.to_le_bytes());
    out.extend((reclen as u16).to_le_bytes());
    out.push(kind);
    out.extend(name);
    out.resize(out.len() + reclen - 19 - name.len(), 0);
}

/// `getdents64(fd, dirp, count)`.
pub(crate) fn getdents64(fd: libc::c_int, dirp: u64, count: u64) -> i64 {
    let Ok(mut dirs) = DIRS.lock() else {
        return EINVAL;
    };
    let dirs = dirs.get_or_insert_with(HashMap::new);
    if let std::collections::hash_map::Entry::Vacant(slot) = dirs.entry(fd) {
        // The stream gets its own descriptor, so that closing it never closes the
        // guest's.
        // SAFETY: dup and fdopendir on the guest's directory descriptor.
        let dir = unsafe {
            let copy = libc::dup(fd);
            if copy < 0 {
                return result(-1);
            }
            let dir = libc::fdopendir(copy);
            if dir.is_null() {
                libc::close(copy);
                return result(-1);
            }
            dir
        };
        slot.insert(Dir(dir));
    }
    let Some(dir) = dirs.get(&fd) else {
        return EINVAL;
    };
    let mut out = Vec::new();
    loop {
        // SAFETY: the stream is open; telldir and readdir have no other
        // requirement.
        let (position, entry) = unsafe { (libc::telldir(dir.0), libc::readdir(dir.0)) };
        if entry.is_null() {
            break;
        }
        // SAFETY: readdir returned a valid entry, until the next call.
        let (ino, kind, name) = unsafe {
            let e = &*entry;
            let name = std::ffi::CStr::from_ptr(e.d_name.as_ptr())
                .to_bytes()
                .to_vec();
            (e.d_ino, e.d_type, name)
        };
        let before = out.len();
        linux_dirent(&mut out, ino, position + 1, kind, &name);
        if out.len() as u64 > count {
            out.truncate(before);
            // SAFETY: back to the entry that did not fit, for the next call.
            unsafe { libc::seekdir(dir.0, position) };
            if before == 0 {
                return EINVAL;
            }
            break;
        }
    }
    // SAFETY: the guest passed `count` bytes; out is no longer than that.
    unsafe { std::ptr::copy_nonoverlapping(out.as_ptr(), dirp as *mut u8, out.len()) };
    out.len() as i64
}

/// Forgets the directory stream of a descriptor being closed.
pub(crate) fn close_dir(fd: libc::c_int) {
    if let Ok(mut dirs) = DIRS.lock() {
        if let Some(dir) = dirs.as_mut().and_then(|d| d.remove(&fd)) {
            // SAFETY: the stream owns the duplicated descriptor.
            unsafe { libc::closedir(dir.0) };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn readable(poll: &mut libc::pollfd) -> i32 {
        // SAFETY: one pollfd, no wait.
        unsafe { libc::poll(poll, 1, 0) }
    }

    #[test]
    fn an_eventfd_counts_and_signals_through_its_pipe() {
        let fd = eventfd(0, EFD_NONBLOCK) as libc::c_int;
        assert!(fd >= 0);
        let mut value = 0u64;
        let at = std::ptr::from_mut(&mut value) as u64;
        assert_eq!(read(fd, at, 8), Some(EAGAIN), "empty and non-blocking");
        let three = 3u64;
        assert_eq!(write(fd, std::ptr::from_ref(&three) as u64, 8), Some(8));
        assert_eq!(write(fd, std::ptr::from_ref(&three) as u64, 8), Some(8));
        let mut poll = libc::pollfd {
            fd,
            events: libc::POLLIN,
            revents: 0,
        };
        assert_eq!(readable(&mut poll), 1, "readable while non-zero");
        assert_eq!(read(fd, at, 8), Some(8));
        assert_eq!(value, 6, "reads return and reset the counter");
        assert_eq!(readable(&mut poll), 0, "and the pipe is drained");
        close(fd);
        // SAFETY: the read end belongs to the test.
        unsafe { libc::close(fd) };
    }

    #[test]
    fn getdents64_lists_a_directory_in_linux_records() {
        let dir = std::env::temp_dir().join(format!("cs2metal-dents-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("mkdir");
        std::fs::write(dir.join("a-file"), b"x").expect("write");
        let path = std::ffi::CString::new(dir.to_string_lossy().as_bytes()).expect("path");
        // SAFETY: opening a directory the test made.
        let fd = unsafe { libc::open(path.as_ptr(), libc::O_RDONLY | libc::O_DIRECTORY) };
        let mut buf = vec![0u8; 4096];
        let n = getdents64(fd, buf.as_mut_ptr() as u64, buf.len() as u64);
        assert!(n > 0);
        let mut names = Vec::new();
        let mut at = 0usize;
        while at < n as usize {
            let reclen = u16::from_le_bytes([buf[at + 16], buf[at + 17]]) as usize;
            let name = &buf[at + 19..at + reclen];
            let end = name.iter().position(|&b| b == 0).unwrap_or(name.len());
            names.push(String::from_utf8_lossy(&name[..end]).into_owned());
            at += reclen;
        }
        assert!(names.iter().any(|n| n == "a-file"));
        assert_eq!(
            getdents64(fd, buf.as_mut_ptr() as u64, buf.len() as u64),
            0,
            "at the end"
        );
        close_dir(fd);
        // SAFETY: the test's own descriptor.
        unsafe { libc::close(fd) };
        let _ = std::fs::remove_dir_all(dir);
    }
}
