//! One reserved range of the address space and the bounds-checked operations the
//! loader performs inside it. Every `unsafe` call of the crate that touches memory
//! is here, behind methods that check offsets against the reservation.

use std::io;
use std::os::fd::{AsRawFd, BorrowedFd};
use std::ptr;

use crate::error::{Error, Result};

/// The host page size.
pub(crate) fn page_size() -> u64 {
    // SAFETY: sysconf has no preconditions and only reads system configuration.
    let size = unsafe { libc::sysconf(libc::_SC_PAGESIZE) };
    u64::try_from(size).unwrap_or(4096)
}

pub(crate) fn round_down(value: u64, page: u64) -> u64 {
    value & !(page - 1)
}

pub(crate) fn round_up(value: u64, page: u64) -> u64 {
    value.saturating_add(page - 1) & !(page - 1)
}

fn last_error() -> Error {
    Error::Map(io::Error::last_os_error())
}

/// A private, anonymous reservation that is unmapped when dropped.
#[derive(Debug)]
pub(crate) struct Region {
    base: *mut u8,
    len: usize,
}

// SAFETY: the region is plain memory owned by this value; nothing ties it to the
// thread that created it.
unsafe impl Send for Region {}

// SAFETY: the methods that write through `&self` run only while the loader builds
// a module, under an exclusive borrow of its namespace or bridge; once built, the
// loader only reads. What guest code does to its own memory is the guest's
// concern, as it is under ld.so.
unsafe impl Sync for Region {}

impl Region {
    /// Reserves `len` bytes, inaccessible until something is mapped there.
    pub fn reserve(len: u64) -> Result<Self> {
        let len =
            usize::try_from(len).map_err(|_| Error::Map(io::ErrorKind::OutOfMemory.into()))?;
        // SAFETY: a fresh anonymous mapping at an address the kernel chooses cannot
        // alias any existing memory.
        let base = unsafe {
            libc::mmap(
                ptr::null_mut(),
                len,
                libc::PROT_NONE,
                libc::MAP_PRIVATE | libc::MAP_ANON,
                -1,
                0,
            )
        };
        if base == libc::MAP_FAILED {
            return Err(last_error());
        }
        Ok(Self {
            base: base.cast(),
            len,
        })
    }

    pub fn address(&self) -> u64 {
        self.base as u64
    }

    fn check(&self, offset: u64, len: u64) -> Result<usize> {
        let end = offset.checked_add(len);
        match end {
            Some(end) if end <= self.len as u64 => Ok(offset as usize),
            _ => Err(Error::RelocationOutOfBounds { offset }),
        }
    }

    /// Maps `len` bytes of `file` from `file_offset` at `offset`, private and
    /// writable, replacing the reservation there.
    pub fn map_file(
        &self,
        offset: u64,
        len: u64,
        file: BorrowedFd<'_>,
        file_offset: u64,
    ) -> Result<()> {
        let at = self.check(offset, len)?;
        let file_offset = libc::off_t::try_from(file_offset)
            .map_err(|_| Error::Map(io::ErrorKind::InvalidInput.into()))?;
        // SAFETY: the target lies inside this reservation (checked above), which
        // only this value owns, so MAP_FIXED replaces nothing anyone else uses.
        let mapped = unsafe {
            libc::mmap(
                self.base.add(at).cast(),
                len as usize,
                libc::PROT_READ | libc::PROT_WRITE,
                libc::MAP_PRIVATE | libc::MAP_FIXED,
                file.as_raw_fd(),
                file_offset,
            )
        };
        if mapped == libc::MAP_FAILED {
            return Err(last_error());
        }
        Ok(())
    }

    /// Maps zeroed, writable memory at `offset`.
    pub fn map_zero(&self, offset: u64, len: u64) -> Result<()> {
        let at = self.check(offset, len)?;
        // SAFETY: as in map_file, the target lies inside this reservation.
        let mapped = unsafe {
            libc::mmap(
                self.base.add(at).cast(),
                len as usize,
                libc::PROT_READ | libc::PROT_WRITE,
                libc::MAP_PRIVATE | libc::MAP_ANON | libc::MAP_FIXED,
                -1,
                0,
            )
        };
        if mapped == libc::MAP_FAILED {
            return Err(last_error());
        }
        Ok(())
    }

    pub fn protect(&self, offset: u64, len: u64, prot: libc::c_int) -> Result<()> {
        let at = self.check(offset, len)?;
        // SAFETY: the range lies inside this reservation; changing its protection
        // affects no memory outside it.
        let status = unsafe { libc::mprotect(self.base.add(at).cast(), len as usize, prot) };
        if status != 0 {
            return Err(last_error());
        }
        Ok(())
    }

    /// Fills `len` bytes at `offset` with zeros. The range must be mapped writable.
    pub fn zero(&self, offset: u64, len: u64) -> Result<()> {
        let at = self.check(offset, len)?;
        // SAFETY: in bounds (checked), and the caller only zeroes ranges it mapped
        // writable before changing any protection.
        unsafe { ptr::write_bytes(self.base.add(at), 0, len as usize) };
        Ok(())
    }

    /// Writes a 64-bit value at `offset`. The page must be mapped writable.
    pub fn write_u64(&self, offset: u64, value: u64) -> Result<()> {
        let at = self.check(offset, 8)?;
        // SAFETY: in bounds (checked); relocations are applied while every segment
        // is still mapped writable; ELF does not promise alignment, hence unaligned.
        unsafe { ptr::write_unaligned(self.base.add(at).cast::<u64>(), value) };
        Ok(())
    }

    /// Reads one byte at `offset`. The page must be mapped readable.
    pub fn read_u8(&self, offset: u64) -> Result<u8> {
        let at = self.check(offset, 1)?;
        // SAFETY: in bounds (checked), and only called on loaded segments.
        Ok(unsafe { ptr::read(self.base.add(at)) })
    }

    /// Writes one byte at `offset`. The page must be mapped writable.
    pub fn write_u8(&self, offset: u64, value: u8) -> Result<()> {
        let at = self.check(offset, 1)?;
        // SAFETY: in bounds (checked), and only called while every segment is
        // still mapped writable, before protections are set.
        unsafe { ptr::write(self.base.add(at), value) };
        Ok(())
    }

    /// Copies `bytes` to `offset`. The range must be mapped writable.
    pub fn write_bytes(&self, offset: u64, bytes: &[u8]) -> Result<()> {
        let at = self.check(offset, bytes.len() as u64)?;
        // SAFETY: in bounds (checked); the source is a separate Rust slice, so the
        // ranges cannot overlap.
        unsafe { ptr::copy_nonoverlapping(bytes.as_ptr(), self.base.add(at), bytes.len()) };
        Ok(())
    }

    /// Reads a 64-bit value at `offset`. The page must be mapped readable.
    pub fn read_u64(&self, offset: u64) -> Result<u64> {
        let at = self.check(offset, 8)?;
        // SAFETY: in bounds (checked), and only called on loaded segments, which are
        // readable.
        Ok(unsafe { ptr::read_unaligned(self.base.add(at).cast::<u64>()) })
    }
}

impl Drop for Region {
    fn drop(&mut self) {
        // SAFETY: base and len describe exactly the mapping this value created, and
        // nothing borrows it past this point.
        unsafe { libc::munmap(self.base.cast(), self.len) };
    }
}
