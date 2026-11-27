//! System V semaphores, on the host's.
//!
//! macOS has `semget`, `semop`, and `semctl`. `struct sembuf` is laid out alike
//! on both, and so are the key, flag, and permission values; `semctl`'s command
//! numbers differ, and macOS has no `semtimedop`, which is answered here by
//! polling with `IPC_NOWAIT` until its timeout.

use crate::translate::{EAGAIN, EINVAL, result};

/// Linux `semctl` commands on the host. `IPC_STAT` and `IPC_SET`, whose
/// structures differ, are not translated.
fn command(linux: u64) -> Option<libc::c_int> {
    Some(match linux & 0xff {
        0 => libc::IPC_RMID,
        11 => libc::GETPID,
        12 => libc::GETVAL,
        13 => libc::GETALL,
        14 => libc::GETNCNT,
        15 => libc::GETZCNT,
        16 => libc::SETVAL,
        17 => libc::SETALL,
        _ => return None,
    })
}

fn semop(id: u64, ops: u64, count: u64) -> i64 {
    // SAFETY: the guest passes `count` sembufs, laid out as the host's.
    result(i64::from(unsafe {
        libc::semop(id as libc::c_int, ops as *mut libc::sembuf, count as usize)
    }))
}

/// `semtimedop(id, ops, count, timeout)`: each operation tried without
/// waiting, again every millisecond, until the timeout.
fn semtimedop(id: u64, ops: u64, count: u64, timeout: u64) -> i64 {
    if timeout == 0 {
        return semop(id, ops, count);
    }
    // SAFETY: the guest passes a struct timespec.
    let [seconds, nanos] = unsafe { std::ptr::read_unaligned(timeout as *const [i64; 2]) };
    let limit = std::time::Duration::new(seconds.max(0) as u64, nanos.clamp(0, 999_999_999) as u32);
    let start = std::time::Instant::now();
    let mut copy: Vec<libc::sembuf> = (0..count)
        // SAFETY: as in semop.
        .map(|i| unsafe { std::ptr::read_unaligned((ops as *const libc::sembuf).add(i as usize)) })
        .collect();
    for op in &mut copy {
        op.sem_flg |= libc::IPC_NOWAIT as libc::c_short;
    }
    loop {
        // SAFETY: the copy holds `count` sembufs.
        let status = unsafe { libc::semop(id as libc::c_int, copy.as_mut_ptr(), copy.len()) };
        if status == 0 {
            return 0;
        }
        let e = result(-1);
        if e != EAGAIN || start.elapsed() >= limit {
            return e;
        }
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
}

/// Handles the system calls of this module.
pub(crate) fn answer(number: u64, a: [u64; 4]) -> Option<i64> {
    Some(match number {
        // SAFETY: plain semget with Linux's key and flags, the host's values.
        64 => result(i64::from(unsafe {
            libc::semget(
                a[0] as libc::key_t,
                a[1] as libc::c_int,
                a[2] as libc::c_int,
            )
        })),
        65 => semop(a[0], a[1], a[2]),
        66 => match command(a[2]) {
            // SAFETY: semctl with the host's command; the argument is an int or
            // a pointer, passed through as the union's word.
            Some(cmd) => result(i64::from(unsafe {
                libc::semctl(a[0] as libc::c_int, a[1] as libc::c_int, cmd, a[3])
            })),
            None => EINVAL,
        },
        220 => semtimedop(a[0], a[1], a[2], a[3]),
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_private_semaphore_counts_and_times_out() {
        const IPC_PRIVATE: u64 = 0;
        const IPC_CREAT: u64 = 0o1000;
        let id = answer(64, [IPC_PRIVATE, 1, IPC_CREAT | 0o600, 0]).expect("semget");
        assert!(id >= 0, "semget gave {id}");
        let id = id as u64;
        assert_eq!(answer(66, [id, 0, 16, 2]), Some(0), "SETVAL 2");
        assert_eq!(answer(66, [id, 0, 12, 0]), Some(2), "GETVAL");
        let down = libc::sembuf {
            sem_num: 0,
            sem_op: -2,
            sem_flg: 0,
        };
        assert_eq!(
            answer(65, [id, std::ptr::from_ref(&down) as u64, 1, 0]),
            Some(0)
        );
        let timeout = [0i64, 20_000_000];
        let started = std::time::Instant::now();
        let again = answer(
            220,
            [
                id,
                std::ptr::from_ref(&down) as u64,
                1,
                timeout.as_ptr() as u64,
            ],
        );
        assert_eq!(again, Some(EAGAIN), "nothing left to take");
        assert!(started.elapsed() >= std::time::Duration::from_millis(20));
        assert_eq!(answer(66, [id, 0, 0, 0]), Some(0), "IPC_RMID");
    }
}
