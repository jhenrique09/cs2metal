use cs2metal_elf::consts::PT_LOAD;
use cs2metal_elf::{Elf, FunctionRange, executable_sections, unwind_ranges};
use iced_x86::{Decoder, DecoderOptions, Instruction, OpKind, Register};

use crate::{FsAccess, ModuleScan, TlsModels};

const R_X86_64_DTPMOD64: u32 = 16;
const R_X86_64_TPOFF64: u32 = 18;
const R_X86_64_TLSDESC: u32 = 36;
const FS_PREFIX: u8 = 0x64;

/// Legacy prefixes that may precede an opcode, REX, VEX, or EVEX byte.
fn is_legacy_prefix(b: u8) -> bool {
    matches!(
        b,
        0x26 | 0x2e | 0x36 | 0x3e | 0x64 | 0x65 | 0x66 | 0x67 | 0xf0 | 0xf2 | 0xf3
    )
}

fn classify(instruction: &Instruction) -> FsAccess {
    // String instructions address through rsi/rdi without a ModRM memory operand;
    // iced then reports no base and a zero displacement, which is not fs:0.
    let addressed = (0..instruction.op_count()).any(|i| instruction.op_kind(i) == OpKind::Memory);
    if !addressed {
        return FsAccess::Unaddressed;
    }
    if instruction.memory_base() != Register::None || instruction.memory_index() != Register::None {
        return FsAccess::RegisterRelative;
    }
    match instruction.memory_displacement64() {
        0 => FsAccess::ThreadPointer,
        0x28 => FsAccess::StackGuard,
        0x30 => FsAccess::PointerGuard,
        _ => FsAccess::ConstantOffset,
    }
}

/// Whether the instruction's bytes carry exactly one `0x64` among their leading
/// legacy prefixes, so that changing it to `0x65` changes nothing else.
fn prefix_located(bytes: &[u8]) -> bool {
    let prefixes = bytes.iter().take_while(|&&b| is_legacy_prefix(b));
    prefixes.filter(|&&b| b == FS_PREFIX).count() == 1
}

/// Merges sorted ranges and clips them to `[lo, hi)`.
fn clip_and_merge(ranges: &[FunctionRange], lo: u64, hi: u64) -> Vec<(u64, u64)> {
    let mut out: Vec<(u64, u64)> = Vec::new();
    for r in ranges {
        let (start, end) = (r.start.max(lo), r.end.min(hi));
        if start >= end {
            continue;
        }
        match out.last_mut() {
            Some(last) if start <= last.1 => last.1 = last.1.max(end),
            _ => out.push((start, end)),
        }
    }
    out
}

struct Tally<'s> {
    scan: &'s mut ModuleScan,
}

impl Tally<'_> {
    fn covered(&mut self, bytes: &[u8], ip: u64) {
        let mut decoder = Decoder::with_ip(64, bytes, ip, DecoderOptions::NONE);
        let mut instruction = Instruction::default();
        while decoder.can_decode() {
            let offset = decoder.position();
            decoder.decode_out(&mut instruction);
            if instruction.is_invalid() {
                self.scan.decode_failures += 1;
                continue;
            }
            if instruction.segment_prefix() == Register::FS {
                *self
                    .scan
                    .fs_accesses
                    .entry(classify(&instruction))
                    .or_default() += 1;
                let end = offset + instruction.len();
                if prefix_located(&bytes[offset..end.min(bytes.len())]) {
                    self.scan.rewritable += 1;
                }
            }
        }
    }

    fn uncovered(&mut self, bytes: &[u8], ip: u64) {
        let mut decoder = Decoder::with_ip(64, bytes, ip, DecoderOptions::NONE);
        let mut instruction = Instruction::default();
        while decoder.can_decode() {
            decoder.decode_out(&mut instruction);
            if !instruction.is_invalid() && instruction.segment_prefix() == Register::FS {
                *self
                    .scan
                    .uncovered_fs
                    .entry(classify(&instruction))
                    .or_default() += 1;
            }
        }
    }
}

/// Scans one parsed module. `data` is the whole file.
pub fn scan_module(data: &[u8], elf: &Elf) -> ModuleScan {
    let counts = &elf.relocations.counts;
    let count = |kind: u32| counts.get(&kind).copied().unwrap_or(0);
    let mut scan = ModuleScan {
        machine: elf.machine_name().to_owned(),
        tls: TlsModels {
            dynamic_module_relocations: count(R_X86_64_DTPMOD64),
            descriptor_relocations: count(R_X86_64_TLSDESC),
            initial_exec_relocations: count(R_X86_64_TPOFF64),
            tls_block_bytes: elf.tls.map_or(0, |t| t.memory_size),
            static_tls_flag: elf.flags.static_tls,
        },
        ..ModuleScan::default()
    };

    let ranges = match unwind_ranges(data, elf) {
        Ok(ranges) => ranges,
        Err(e) => {
            scan.unwind_error = Some(e.to_string());
            Vec::new()
        }
    };
    scan.functions = ranges.len();

    // Code is the executable sections when the object names them; otherwise all of
    // each executable segment, which may include data mapped with it.
    let sections = match executable_sections(data) {
        Ok(Some(list)) => Some(list),
        Ok(None) => None,
        Err(e) => {
            scan.section_error = Some(e.to_string());
            None
        }
    };
    scan.code_from_sections = sections.is_some();

    let mut tally = Tally { scan: &mut scan };
    let executable = elf
        .segments
        .iter()
        .filter(|s| s.kind == PT_LOAD && s.executable());
    for segment in executable {
        let start = usize::try_from(segment.offset).unwrap_or(usize::MAX);
        let len = usize::try_from(segment.file_size).unwrap_or(0);
        let Some(bytes) = start.checked_add(len).and_then(|end| data.get(start..end)) else {
            continue;
        };
        let (lo, hi) = (segment.vaddr, segment.vaddr + segment.file_size);
        let regions = match &sections {
            Some(list) => list
                .iter()
                .map(|s| (s.start.max(lo), s.end.min(hi)))
                .filter(|(a, b)| a < b)
                .collect(),
            None => vec![(lo, hi)],
        };
        let slice = |a: u64, b: u64| &bytes[(a - lo) as usize..(b - lo) as usize];
        for (rlo, rhi) in regions {
            tally.scan.code_bytes += rhi - rlo;
            let mut cursor = rlo;
            for (start, end) in clip_and_merge(&ranges, rlo, rhi) {
                if cursor < start {
                    tally.uncovered(slice(cursor, start), cursor);
                }
                tally.covered(slice(start, end), start);
                tally.scan.covered_bytes += end - start;
                cursor = end;
            }
            if cursor < rhi {
                tally.uncovered(slice(cursor, rhi), cursor);
            }
        }
    }
    scan
}
