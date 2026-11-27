//! Executable sections, from the section header table.
//!
//! A loader never needs section headers, and a stripped object may lack them. They
//! matter to analysis: some linkers put headers, string tables, `.rodata`, and
//! `.eh_frame` in the same executable `PT_LOAD` segment as `.text`, and only the
//! section table tells code from the data mapped next to it.

use crate::consts::{SHDR_SIZE, SHF_ALLOC, SHF_EXECINSTR, SHT_NOBITS};
use crate::error::{Error, Result};
use crate::reader::{Reader, offset_of};

/// Upper bound on the section headers read, against a corrupted count.
const MAX_SECTIONS: u64 = 1 << 16;

/// One allocated section's virtual address range `[start, end)`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, serde::Serialize)]
pub struct SectionRange {
    pub start: u64,
    pub end: u64,
}

/// The allocated, executable sections with file contents (`SHF_ALLOC |
/// SHF_EXECINSTR`, not `SHT_NOBITS`), sorted by address. `None` when the object has
/// no section header table.
pub fn executable_sections(data: &[u8]) -> Result<Option<Vec<SectionRange>>> {
    let r = Reader::new(data);
    let shoff = r.u64(40)?;
    let shentsize = u64::from(r.u16(58)?);
    let mut shnum = u64::from(r.u16(60)?);
    if shoff == 0 {
        return Ok(None);
    }
    if shentsize != SHDR_SIZE {
        return Err(Error::BadEntrySize {
            table: "section header",
            size: shentsize,
        });
    }
    if shnum == 0 {
        // Extended numbering: the count is in the first header's sh_size.
        shnum = r.u64(offset_of(shoff, 0, SHDR_SIZE)? + 32)?;
    }
    let mut out = Vec::new();
    for i in 0..shnum.min(MAX_SECTIONS) {
        let at = offset_of(shoff, i, SHDR_SIZE)?;
        let kind = r.u32(at + 4)?;
        let flags = r.u64(at + 8)?;
        let addr = r.u64(at + 16)?;
        let size = r.u64(at + 32)?;
        let wanted = SHF_ALLOC | SHF_EXECINSTR;
        if flags & wanted == wanted && kind != SHT_NOBITS && size > 0 {
            let end = addr.checked_add(size).ok_or(Error::OutOfBounds {
                offset: at,
                len: SHDR_SIZE,
            })?;
            out.push(SectionRange { start: addr, end });
        }
    }
    out.sort();
    Ok(Some(out))
}
