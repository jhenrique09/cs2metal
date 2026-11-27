//! Function ranges from the `.eh_frame` unwind tables.
//!
//! The tables are found the way unwinders find them at run time: through the
//! `PT_GNU_EH_FRAME` segment, whose header (`.eh_frame_hdr`) points at `.eh_frame`.
//! Every frame description entry (FDE) there names one range of code. Section
//! headers are not needed.

use crate::consts::{PT_GNU_EH_FRAME, PT_LOAD};
use crate::error::{Error, Result};
use crate::reader::Reader;
use crate::{Elf, Segment};

/// The code range one FDE describes, as virtual addresses `[start, end)`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, serde::Serialize)]
pub struct FunctionRange {
    pub start: u64,
    pub end: u64,
}

const DW_EH_PE_OMIT: u8 = 0xff;
const DW_EH_PE_PCREL: u8 = 0x10;
const DW_EH_PE_DATAREL: u8 = 0x30;
const DW_EH_PE_INDIRECT: u8 = 0x80;

/// Upper bound on the entries walked, against a corrupted length field.
const MAX_ENTRIES: usize = 1 << 22;

/// A cursor over file bytes that knows the virtual address of each position.
struct Cursor<'a> {
    r: Reader<'a>,
    offset: u64,
    /// Virtual address minus file offset for the segment being read.
    delta: i128,
    /// End of the readable range, as a file offset.
    limit: u64,
}

impl Cursor<'_> {
    fn vaddr(&self) -> u64 {
        (i128::from(self.offset) + self.delta) as u64
    }

    fn check(&self, len: u64) -> Result<()> {
        if self.offset.saturating_add(len) > self.limit {
            return Err(Error::BadUnwindInfo("read past the end of the segment"));
        }
        Ok(())
    }

    fn u8(&mut self) -> Result<u8> {
        self.check(1)?;
        let v = self.r.u8(self.offset)?;
        self.offset += 1;
        Ok(v)
    }

    fn u16(&mut self) -> Result<u16> {
        self.check(2)?;
        let v = self.r.u16(self.offset)?;
        self.offset += 2;
        Ok(v)
    }

    fn u32(&mut self) -> Result<u32> {
        self.check(4)?;
        let v = self.r.u32(self.offset)?;
        self.offset += 4;
        Ok(v)
    }

    fn u64(&mut self) -> Result<u64> {
        self.check(8)?;
        let v = self.r.u64(self.offset)?;
        self.offset += 8;
        Ok(v)
    }

    fn uleb(&mut self) -> Result<u64> {
        let mut value = 0u64;
        for shift in (0..64).step_by(7) {
            let byte = self.u8()?;
            value |= u64::from(byte & 0x7f) << shift;
            if byte & 0x80 == 0 {
                return Ok(value);
            }
        }
        Err(Error::BadUnwindInfo("LEB128 value too long"))
    }

    fn sleb(&mut self) -> Result<i64> {
        let mut value = 0i64;
        let mut shift = 0;
        loop {
            let byte = self.u8()?;
            value |= i64::from(byte & 0x7f) << shift;
            shift += 7;
            if byte & 0x80 == 0 {
                if shift < 64 && byte & 0x40 != 0 {
                    value |= -1i64 << shift;
                }
                return Ok(value);
            }
            if shift >= 64 {
                return Err(Error::BadUnwindInfo("LEB128 value too long"));
            }
        }
    }

    fn cstr(&mut self) -> Result<String> {
        let s = self.r.cstr(self.offset)?;
        self.check(s.len() as u64 + 1)?;
        self.offset += s.len() as u64 + 1;
        Ok(s)
    }

    /// Reads a pointer in DWARF exception-header encoding. `datarel_base` is the
    /// base for `DW_EH_PE_datarel`, which only `.eh_frame_hdr` uses.
    fn pointer(&mut self, encoding: u8, datarel_base: u64) -> Result<u64> {
        if encoding == DW_EH_PE_OMIT {
            return Err(Error::BadUnwindInfo("omitted pointer"));
        }
        if encoding & DW_EH_PE_INDIRECT != 0 {
            return Err(Error::BadUnwindInfo("indirect pointer encoding"));
        }
        let field = self.vaddr();
        let raw = self.value(encoding & 0x0f)?;
        let base = match encoding & 0x70 {
            0 => 0,
            DW_EH_PE_PCREL => field,
            DW_EH_PE_DATAREL => datarel_base,
            _ => return Err(Error::BadUnwindInfo("unsupported pointer application")),
        };
        Ok(base.wrapping_add(raw as u64))
    }

    /// Reads a value in the format of the low nibble of an encoding.
    fn value(&mut self, format: u8) -> Result<i64> {
        Ok(match format {
            0x00 | 0x04 => self.u64()? as i64,
            0x01 => self.uleb()? as i64,
            0x02 => i64::from(self.u16()?),
            0x03 => i64::from(self.u32()?),
            0x09 => self.sleb()?,
            0x0a => i64::from(self.u16()? as i16),
            0x0b => i64::from(self.u32()? as i32),
            0x0c => self.u64()? as i64,
            _ => return Err(Error::BadUnwindInfo("unsupported pointer format")),
        })
    }
}

fn load_containing(elf: &Elf, vaddr: u64) -> Option<&Segment> {
    elf.segments
        .iter()
        .filter(|s| s.kind == PT_LOAD)
        .find(|s| vaddr >= s.vaddr && vaddr - s.vaddr < s.file_size)
}

/// The FDE pointer encoding a CIE declares, `DW_EH_PE_absptr` when it has none.
fn cie_pointer_encoding(c: &mut Cursor<'_>, end: u64) -> Result<u8> {
    let version = c.u8()?;
    let augmentation = c.cstr()?;
    if augmentation.contains("eh") {
        return Err(Error::BadUnwindInfo("obsolete \"eh\" augmentation"));
    }
    c.uleb()?; // code alignment
    c.sleb()?; // data alignment
    if version == 1 {
        c.u8()?;
    } else {
        c.uleb()?;
    } // return address register
    let mut encoding = 0u8;
    if let Some(rest) = augmentation.strip_prefix('z') {
        c.uleb()?; // augmentation data length
        for ch in rest.chars() {
            match ch {
                'R' => encoding = c.u8()?,
                'L' => {
                    c.u8()?;
                }
                'P' => {
                    // The personality routine is only skipped, so an indirect
                    // encoding (common in C++ runtimes) needs no dereference.
                    let personality = c.u8()?;
                    if personality != DW_EH_PE_OMIT {
                        c.value(personality & 0x0f)?;
                    }
                }
                'S' | 'B' | 'G' => {}
                _ => break,
            }
        }
    }
    if c.offset > end {
        return Err(Error::BadUnwindInfo("CIE overruns its length"));
    }
    Ok(encoding)
}

/// Every code range the `.eh_frame` tables describe, sorted by start address.
/// An object without `PT_GNU_EH_FRAME` yields no ranges.
pub fn unwind_ranges(data: &[u8], elf: &Elf) -> Result<Vec<FunctionRange>> {
    let Some(hdr) = elf.segments.iter().find(|s| s.kind == PT_GNU_EH_FRAME) else {
        return Ok(Vec::new());
    };
    let r = Reader::new(data);
    let hdr_load = load_containing(elf, hdr.vaddr).ok_or(Error::UnmappedAddress(hdr.vaddr))?;
    let mut c = Cursor {
        r,
        offset: hdr.offset,
        delta: i128::from(hdr_load.vaddr) - i128::from(hdr_load.offset),
        limit: hdr_load.offset + hdr_load.file_size,
    };
    if c.u8()? != 1 {
        return Err(Error::BadUnwindInfo("unknown .eh_frame_hdr version"));
    }
    let frame_ptr_encoding = c.u8()?;
    c.u8()?; // FDE count encoding
    c.u8()?; // table encoding
    let eh_frame = c.pointer(frame_ptr_encoding, hdr.vaddr)?;

    let load = load_containing(elf, eh_frame).ok_or(Error::UnmappedAddress(eh_frame))?;
    let mut c = Cursor {
        r,
        offset: load.offset + (eh_frame - load.vaddr),
        delta: i128::from(load.vaddr) - i128::from(load.offset),
        limit: load.offset + load.file_size,
    };

    let mut encodings: Vec<(u64, u8)> = Vec::new();
    let mut ranges = Vec::new();
    for _ in 0..MAX_ENTRIES {
        if c.offset + 4 > c.limit {
            break;
        }
        let start = c.offset;
        let mut length = u64::from(c.u32()?);
        if length == 0 {
            break; // terminator
        }
        if length == 0xffff_ffff {
            length = c.u64()?;
        }
        let body = c.offset;
        let end = body
            .checked_add(length)
            .filter(|&e| e <= c.limit)
            .ok_or(Error::BadUnwindInfo("entry overruns the segment"))?;
        let id = c.u32()?;
        if id == 0 {
            let encoding = cie_pointer_encoding(&mut c, end)?;
            encodings.push((start, encoding));
        } else {
            let cie = body
                .checked_sub(u64::from(id))
                .ok_or(Error::BadUnwindInfo("CIE pointer before the table"))?;
            let encoding = encodings
                .iter()
                .find(|(at, _)| *at == cie)
                .map(|(_, e)| *e)
                .ok_or(Error::BadUnwindInfo("FDE names an unknown CIE"))?;
            let pc_begin = c.pointer(encoding, 0)?;
            let pc_range = c.value(encoding & 0x0f)? as u64;
            if pc_range > 0 {
                ranges.push(FunctionRange {
                    start: pc_begin,
                    end: pc_begin.saturating_add(pc_range),
                });
            }
        }
        c.offset = end;
    }
    ranges.sort();
    Ok(ranges)
}
