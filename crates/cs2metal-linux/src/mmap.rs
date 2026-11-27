//! `mmap` for the guest, with transform T-001 applied to every executable file
//! mapping: the private copy of a module's code gets its proven `fs` prefixes
//! rewritten to `gs` before the guest sees it.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use cs2metal_elf::consts::PT_LOAD;

use crate::translate::{self, EACCES};

/// The file offsets of a module's rewrite sites, and what was left alone.
#[derive(Debug, Default)]
pub(crate) struct Sites {
    offsets: Vec<u64>,
    unexplained: u64,
}

/// Device and inode.
type FileId = (u64, u64);

/// Rewrite sites per file, computed once.
static SITES: Mutex<Option<HashMap<FileId, Arc<Sites>>>> = Mutex::new(None);

/// What T-001 did, for the log.
pub(crate) static REWRITTEN: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

fn read_file(fd: libc::c_int, size: usize) -> Option<Vec<u8>> {
    let mut data = vec![0u8; size];
    let mut done = 0;
    while done < size {
        // SAFETY: pread writes at most size - done bytes into the buffer's tail.
        let n = unsafe {
            libc::pread(
                fd,
                data[done..].as_mut_ptr().cast(),
                size - done,
                done as libc::off_t,
            )
        };
        if n <= 0 {
            return None;
        }
        done += n as usize;
    }
    Some(data)
}

/// The rewrite sites of the file open at `fd`, as file offsets.
fn sites(fd: libc::c_int) -> Option<Arc<Sites>> {
    // SAFETY: fstat writes into the zeroed struct.
    let mut st: libc::stat = unsafe { std::mem::zeroed() };
    // SAFETY: as above.
    if unsafe { libc::fstat(fd, &mut st) } != 0 {
        return None;
    }
    let key = (st.st_dev as u64, st.st_ino);
    if let Ok(cache) = SITES.lock() {
        if let Some(found) = cache.as_ref().and_then(|c| c.get(&key)) {
            return Some(found.clone());
        }
    }
    let data = read_file(fd, usize::try_from(st.st_size).ok()?)?;
    let sites = match cs2metal_elf::parse(&data) {
        Ok(elf) => {
            let scan = cs2metal_codescan::scan_module(&data, &elf);
            let loads: Vec<_> = elf.segments.iter().filter(|s| s.kind == PT_LOAD).collect();
            let offsets = scan
                .rewrite_sites
                .iter()
                .filter_map(|&v| {
                    loads
                        .iter()
                        .find(|s| v >= s.vaddr && v < s.vaddr + s.file_size)
                        .map(|s| v - s.vaddr + s.offset)
                })
                .collect();
            Sites {
                offsets,
                unexplained: scan.unexplained_addressed(),
            }
        }
        Err(_) => Sites::default(),
    };
    let sites = Arc::new(sites);
    if let Ok(mut cache) = SITES.lock() {
        cache
            .get_or_insert_with(HashMap::new)
            .insert(key, sites.clone());
    }
    Some(sites)
}

/// Applies T-001 to `len` bytes mapped at `addr` from file offset `offset`.
fn rewrite(fd: libc::c_int, addr: u64, len: u64, offset: u64, prot: libc::c_int) -> i64 {
    let Some(sites) = sites(fd) else {
        return 0;
    };
    if sites.unexplained > 0 {
        eprintln!(
            "cs2metal: refusing an executable mapping with {} unexplained fs decodes",
            sites.unexplained
        );
        return EACCES;
    }
    let inside: Vec<u64> = sites
        .offsets
        .iter()
        .filter(|&&o| o >= offset && o < offset + len)
        .map(|&o| addr + (o - offset))
        .collect();
    if inside.is_empty() {
        return 0;
    }
    // SAFETY: the range is the private mapping just made for the guest.
    let writable = unsafe {
        libc::mprotect(
            addr as *mut _,
            len as usize,
            libc::PROT_READ | libc::PROT_WRITE,
        )
    };
    if writable != 0 {
        return translate::result(-1);
    }
    for at in &inside {
        // SAFETY: each site lies inside the mapping, now writable; the scan
        // located a 0x64 prefix byte there, which is checked before writing.
        unsafe {
            let byte = *at as *mut u8;
            if *byte == 0x64 {
                *byte = 0x65;
            }
        }
    }
    REWRITTEN.fetch_add(inside.len() as u64, std::sync::atomic::Ordering::Relaxed);
    // SAFETY: as above; back to what the guest asked for.
    unsafe { libc::mprotect(addr as *mut _, len as usize, prot) };
    0
}

/// `mmap(addr, len, prot, flags, fd, offset)` with Linux values.
pub(crate) fn mmap(addr: u64, len: u64, prot: u64, flags: u64, fd: i64, offset: u64) -> i64 {
    let prot = (prot & 0x7) as libc::c_int;
    let mac_flags = translate::mmap_flags(flags);
    let anonymous = mac_flags & libc::MAP_ANON != 0;
    let fd = if anonymous { -1 } else { fd as libc::c_int };
    // SAFETY: a mapping the guest asked for, at its address if it gave MAP_FIXED;
    // the guest owns its address space as a Linux process does.
    let mapped = unsafe {
        libc::mmap(
            addr as *mut _,
            len as usize,
            prot,
            mac_flags,
            fd,
            offset as libc::off_t,
        )
    };
    if mapped == libc::MAP_FAILED {
        return translate::result(-1);
    }
    let mapped = mapped as u64;
    if !anonymous && prot & libc::PROT_EXEC != 0 {
        let status = rewrite(fd, mapped, len, offset, prot);
        if status < 0 {
            // SAFETY: undoes the mapping made above.
            unsafe { libc::munmap(mapped as *mut _, len as usize) };
            return status;
        }
    }
    mapped as i64
}
