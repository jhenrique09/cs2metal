use cs2metal_elf::consts::PT_LOAD;
use cs2metal_elf::{Elf, FunctionRange, executable_sections, unwind_ranges};
use iced_x86::{Decoder, DecoderOptions, Instruction};

use crate::discover::{Discoverer, Region};
use crate::fs::{access, prefix_located};
use crate::{ModuleScan, TlsModels};

const R_X86_64_DTPMOD64: u32 = 16;
const R_X86_64_TPOFF64: u32 = 18;
const R_X86_64_TLSDESC: u32 = 36;
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

/// Counts, by kind, the `fs` accesses a linear sweep finds in one run of
/// unexplained bytes. Those at or after a known data start in the run lie in data
/// that code loads by address; the others are unexplained.
fn sweep(bytes: &[u8], ip: u64, data_starts: &[u64], scan: &mut ModuleScan) {
    let end = ip + bytes.len() as u64;
    let first_data = data_starts
        .get(data_starts.partition_point(|&d| d < ip))
        .copied()
        .filter(|&d| d < end);
    let mut decoder = Decoder::with_ip(64, bytes, ip, DecoderOptions::NONE);
    let mut instruction = Instruction::default();
    while decoder.can_decode() {
        decoder.decode_out(&mut instruction);
        if let Some(kind) = access(&instruction).filter(|_| !instruction.is_invalid()) {
            let in_data = first_data.is_some_and(|d| instruction.ip() >= d);
            let counts = if in_data {
                &mut scan.data_fs
            } else {
                &mut scan.uncovered_fs
            };
            *counts.entry(kind).or_default() += 1;
        }
    }
}

/// The code regions of a module: its executable sections inside executable
/// segments, or the whole segments when it has no section table.
fn code_regions<'a>(data: &'a [u8], elf: &Elf, scan: &mut ModuleScan) -> Vec<Region<'a>> {
    let sections = match executable_sections(data) {
        Ok(list) => list,
        Err(e) => {
            scan.section_error = Some(e.to_string());
            None
        }
    };
    scan.code_from_sections = sections.is_some();
    let mut regions = Vec::new();
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
        let bounds: Vec<(u64, u64)> = match &sections {
            Some(list) => list
                .iter()
                .map(|s| (s.start.max(lo), s.end.min(hi)))
                .filter(|(a, b)| a < b)
                .collect(),
            None => vec![(lo, hi)],
        };
        for (a, b) in bounds {
            regions.push(Region::new(a, &bytes[(a - lo) as usize..(b - lo) as usize]));
        }
    }
    regions
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
            executable: elf.interpreter.is_some(),
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

    let regions = code_regions(data, elf, &mut scan);
    let bounds: Vec<(u64, u64)> = regions
        .iter()
        .map(|r| (r.lo, r.lo + r.bytes.len() as u64))
        .collect();
    let mut discoverer = Discoverer::new(regions, elf);

    // Unwind-described code first: T-001 rewrites it without further proof.
    for (lo, hi) in bounds {
        scan.code_bytes += hi - lo;
        for (start, end) in clip_and_merge(&ranges, lo, hi) {
            scan.covered_bytes += end - start;
            let (accesses, rewritable) = (&mut scan.fs_accesses, &mut scan.rewritable);
            scan.decode_failures += discoverer.unwind_range(start, end, |i, bytes| {
                if let Some(kind) = access(i) {
                    *accesses.entry(kind).or_default() += 1;
                    *rewritable += u64::from(prefix_located(bytes));
                }
            });
        }
    }

    discoverer.seed_from_module();
    discoverer.run();

    let (regions, discovery) = discoverer.into_parts();
    for region in &regions {
        for (start, end) in region.unexplained() {
            let at = (start - region.lo) as usize..(end - region.lo) as usize;
            sweep(&region.bytes[at], start, &discovery.data_starts, &mut scan);
        }
    }
    scan.discovery = discovery;
    scan
}
