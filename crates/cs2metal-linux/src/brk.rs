//! `brk`: a program break in a reserved range.
//!
//! glibc's malloc grows its main arena with `brk`, and when that fails it maps a
//! separate region for every extension. The host reserves 1 GiB of address space
//! once, inaccessible, and makes it readable and writable as the break moves, as
//! the Linux kernel grows a heap.

use std::sync::Mutex;

const RESERVE: u64 = 1 << 30;
const PAGE: u64 = 4096;

struct Heap {
    start: u64,
    /// The current break.
    end: u64,
    /// How far the range is accessible, page-rounded.
    committed: u64,
}

static HEAP: Mutex<Option<Heap>> = Mutex::new(None);

fn reserve() -> Option<Heap> {
    // SAFETY: a fresh anonymous reservation the kernel places.
    let base = unsafe {
        libc::mmap(
            std::ptr::null_mut(),
            RESERVE as usize,
            libc::PROT_NONE,
            libc::MAP_PRIVATE | libc::MAP_ANON,
            -1,
            0,
        )
    };
    (base != libc::MAP_FAILED).then(|| {
        let start = base as u64;
        Heap {
            start,
            end: start,
            committed: start,
        }
    })
}

/// `brk(addr)`: returns the new break, or the current one if `addr` is 0 or
/// cannot be reached, as the kernel does.
pub(crate) fn brk(addr: u64) -> i64 {
    let Ok(mut guard) = HEAP.lock() else {
        return 0;
    };
    if guard.is_none() {
        *guard = reserve();
    }
    let Some(heap) = guard.as_mut() else {
        return 0;
    };
    if addr == 0 || addr < heap.start || addr > heap.start + RESERVE {
        return heap.end as i64;
    }
    let wanted = addr.div_ceil(PAGE) * PAGE;
    if wanted > heap.committed {
        // SAFETY: the pages lie inside the reservation this module owns.
        let status = unsafe {
            libc::mprotect(
                heap.committed as *mut _,
                (wanted - heap.committed) as usize,
                libc::PROT_READ | libc::PROT_WRITE,
            )
        };
        if status != 0 {
            return heap.end as i64;
        }
        heap.committed = wanted;
    }
    heap.end = addr;
    heap.end as i64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_break_grows_into_writable_memory() {
        let start = brk(0) as u64;
        assert_ne!(start, 0);
        assert_eq!(brk(start + 100_000) as u64, start + 100_000);
        // SAFETY: the break just moved past this byte.
        unsafe { std::ptr::write((start + 99_999) as *mut u8, 7) };
        assert_eq!(
            brk(start + RESERVE + 1) as u64,
            start + 100_000,
            "beyond the reserve"
        );
        assert_eq!(brk(0) as u64, start + 100_000);
    }
}
