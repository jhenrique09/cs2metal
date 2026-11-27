//! Code discovery: which bytes of a module are instructions.
//!
//! The ranges the `.eh_frame` tables describe are decoded linearly first. From
//! there, and from the other places a module names its own code (exported
//! functions, the entry point, `DT_INIT` and `DT_FINI`, and code pointers its
//! relocations write), discovery follows direct calls and jumps by recursive
//! descent. It stops at returns, indirect jumps, traps, and calls to functions known
//! not to return. Those paths are *trusted*: every seed is a code address by the
//! module's own declaration.
//!
//! A `lea` with a RIP-relative operand also names code when it loads a callback,
//! but it names data just as often: some hand-written code keeps its tables in
//! `.text`. Such a target is followed *tentatively*, together with everything it
//! reaches, and the whole component is discarded if any of it overlaps code already
//! decoded, fails to decode, or is an `fs` string instruction, which compilers do
//! not emit. Discarded components are counted, never hidden.
//!
//! A byte is code only if an accepted path reaches it; every other byte of the
//! executable sections stays unexplained.

use std::collections::{BTreeMap, HashMap};

use cs2metal_elf::consts::{
    R_X86_64_64, R_X86_64_GLOB_DAT, R_X86_64_IRELATIVE, R_X86_64_JUMP_SLOT, R_X86_64_RELATIVE,
};
use cs2metal_elf::{Elf, SymbolKind};
use iced_x86::{Code, Decoder, DecoderOptions, FlowControl, Instruction, Mnemonic, OpKind};
use serde::Serialize;

use crate::FsAccess;
use crate::fs::{access, prefix_offset};

/// Where a discovery path started.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Seed {
    /// A direct call or jump target found in code the unwind tables describe.
    UnwindCode,
    /// A defined function symbol in the dynamic symbol table.
    Export,
    /// The ELF entry point.
    Entry,
    /// `DT_INIT` or `DT_FINI`.
    Initializer,
    /// A relocation that writes a code address: a vtable slot, an initializer array
    /// entry, an IFUNC resolver.
    CodePointer,
    /// A `lea` with a RIP-relative operand, in accepted code, whose target was
    /// accepted as code: a callback handed to a thread, atexit, or TLS key API.
    AddressTaken,
}

impl Seed {
    pub const ALL: [Self; 6] = [
        Self::UnwindCode,
        Self::Export,
        Self::Entry,
        Self::Initializer,
        Self::CodePointer,
        Self::AddressTaken,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::UnwindCode => "branch from unwind-described code",
            Self::Export => "exported function",
            Self::Entry => "entry point",
            Self::Initializer => "DT_INIT or DT_FINI",
            Self::CodePointer => "code pointer in a relocation",
            Self::AddressTaken => "lea target accepted as code",
        }
    }
}

/// What discovery found beyond the unwind tables.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Discovery {
    /// Seeds that pointed into unexplored code, by source.
    pub seeds: BTreeMap<Seed, u64>,
    /// Bytes of accepted instructions outside the unwind-described ranges.
    pub reached_bytes: u64,
    pub instructions: u64,
    /// `fs` accesses in accepted code outside the unwind-described ranges.
    pub fs_accesses: BTreeMap<FsAccess, u64>,
    /// Those whose `0x64` prefix byte was located.
    pub rewritable: u64,
    /// Trusted paths stopped because an instruction would overlap one already
    /// decoded.
    pub conflicts: u64,
    /// Trusted paths stopped at an encoding that does not decode.
    pub invalid: u64,
    /// Calls not followed past because the callee does not return.
    pub noreturn_calls: u64,
    /// Tentative components discarded as data, and their bytes.
    pub rejected: u64,
    pub rejected_bytes: u64,
    /// Where each discarded component started: an address accepted code loads
    /// with `lea` and that does not decode as code. Sorted; not serialized.
    #[serde(skip)]
    pub data_starts: Vec<u64>,
    /// Addresses of the located prefix bytes in discovered code. Not serialized.
    #[serde(skip)]
    pub sites: Vec<u64>,
}

impl Discovery {
    pub fn fs_total(&self) -> u64 {
        self.fs_accesses.values().sum()
    }
}

/// Functions that never return, by the name a module imports or exports.
fn is_noreturn(name: &str) -> bool {
    const NAMES: [&str; 27] = [
        "abort",
        "exit",
        "_exit",
        "_Exit",
        "quick_exit",
        "__assert_fail",
        "__stack_chk_fail",
        "__fortify_fail",
        "__chk_fail",
        "__cxa_throw",
        "__cxa_rethrow",
        "__cxa_bad_cast",
        "__cxa_bad_typeid",
        "__cxa_throw_bad_array_new_length",
        "__cxa_pure_virtual",
        "__cxa_deleted_virtual",
        "_Unwind_Resume",
        "longjmp",
        "_longjmp",
        "siglongjmp",
        "__longjmp_chk",
        "pthread_exit",
        "err",
        "errx",
        "verr",
        "verrx",
        "_ZSt9terminatev",
    ];
    NAMES.contains(&name)
        || (name.starts_with("_ZSt") && (name.contains("__throw_") || name.contains("rethrow")))
}

const UNKNOWN: u8 = 0;
const START: u8 = 1;
const BODY: u8 = 2;
const FDE_START: u8 = 3;
const FDE_BODY: u8 = 4;

/// One executable section (or segment) and the state of each of its bytes.
pub(crate) struct Region<'a> {
    pub lo: u64,
    pub bytes: &'a [u8],
    state: Vec<u8>,
}

impl<'a> Region<'a> {
    pub fn new(lo: u64, bytes: &'a [u8]) -> Self {
        Self {
            lo,
            bytes,
            state: vec![UNKNOWN; bytes.len()],
        }
    }

    fn hi(&self) -> u64 {
        self.lo + self.bytes.len() as u64
    }

    /// Maximal runs `[start, end)` of bytes no accepted path reached.
    pub fn unexplained(&self) -> Vec<(u64, u64)> {
        let mut out = Vec::new();
        let mut run: Option<usize> = None;
        for (i, &s) in self.state.iter().enumerate() {
            match (s == UNKNOWN, run) {
                (true, None) => run = Some(i),
                (false, Some(start)) => {
                    out.push((self.lo + start as u64, self.lo + i as u64));
                    run = None;
                }
                _ => {}
            }
        }
        if let Some(start) = run {
            out.push((self.lo + start as u64, self.hi()));
        }
        out
    }
}

/// What one component of paths found, held back until it is accepted.
#[derive(Default)]
struct Component {
    marks: Vec<(usize, usize, usize)>,
    bytes: u64,
    instructions: u64,
    /// Each `fs` access and the address of its prefix byte, when located.
    fs: Vec<(FsAccess, Option<u64>)>,
    noreturn_calls: u64,
    conflicts: u64,
    invalid: u64,
    address_taken: Vec<u64>,
    data: bool,
}

pub(crate) struct Discoverer<'a, 'e> {
    regions: Vec<Region<'a>>,
    elf: &'e Elf,
    /// Trusted seeds.
    work: Vec<u64>,
    /// `lea` targets, to be tried one component at a time.
    pending: Vec<u64>,
    /// GOT slot or function address to the name of the function behind it.
    names: HashMap<u64, &'e str>,
    callee_cache: HashMap<u64, bool>,
    out: Discovery,
}

impl<'a, 'e> Discoverer<'a, 'e> {
    pub fn new(mut regions: Vec<Region<'a>>, elf: &'e Elf) -> Self {
        regions.sort_by_key(|r| r.lo);
        let mut names = HashMap::new();
        for r in &elf.relocations.entries {
            if matches!(r.kind, R_X86_64_JUMP_SLOT | R_X86_64_GLOB_DAT) {
                if let Some(s) = elf.relocation_symbol(r) {
                    names.insert(r.offset, s.name.as_str());
                }
            }
        }
        for s in elf.exports() {
            if matches!(s.kind, SymbolKind::Func | SymbolKind::GnuIfunc) {
                names.insert(s.value, s.name.as_str());
            }
        }
        Self {
            regions,
            elf,
            work: Vec::new(),
            pending: Vec::new(),
            names,
            callee_cache: HashMap::new(),
            out: Discovery::default(),
        }
    }

    fn locate(&self, addr: u64) -> Option<(usize, usize)> {
        let i = self
            .regions
            .partition_point(|r| r.lo <= addr)
            .checked_sub(1)?;
        let r = &self.regions[i];
        (addr < r.hi()).then(|| (i, (addr - r.lo) as usize))
    }

    fn state(&self, addr: u64) -> Option<u8> {
        self.locate(addr).map(|(i, o)| self.regions[i].state[o])
    }

    /// Decodes an unwind-described range linearly, marks it, and queues the targets
    /// it names. `visit` sees each instruction and its bytes. Returns the decoder
    /// failures inside the range.
    pub fn unwind_range(
        &mut self,
        start: u64,
        end: u64,
        mut visit: impl FnMut(&Instruction, &[u8]),
    ) -> u64 {
        let Some((ri, off)) = self.locate(start) else {
            return 0;
        };
        let len = (end - start) as usize;
        let mut failures = 0;
        let mut direct = Vec::new();
        {
            let region = &mut self.regions[ri];
            let bytes = &region.bytes[off..off + len];
            let mut decoder = Decoder::with_ip(64, bytes, start, DecoderOptions::NONE);
            let mut instruction = Instruction::default();
            while decoder.can_decode() {
                let pos = decoder.position();
                decoder.decode_out(&mut instruction);
                if instruction.is_invalid() {
                    failures += 1;
                    continue;
                }
                let end = (pos + instruction.len()).min(len);
                region.state[off + pos] = FDE_START;
                region.state[off + pos + 1..off + end].fill(FDE_BODY);
                visit(&instruction, &bytes[pos..end]);
                direct.extend(direct_target(&instruction));
                self.pending.extend(address_taken(&instruction));
            }
        }
        for t in direct {
            if self.state(t) == Some(UNKNOWN) {
                *self.out.seeds.entry(Seed::UnwindCode).or_default() += 1;
                self.work.push(t);
            }
        }
        failures
    }

    fn seed(&mut self, addr: u64, kind: Seed) {
        if self.state(addr) == Some(UNKNOWN) {
            *self.out.seeds.entry(kind).or_default() += 1;
            self.work.push(addr);
        }
    }

    /// Queues the module's own references to its code.
    pub fn seed_from_module(&mut self) {
        let elf = self.elf;
        for s in elf.exports() {
            if matches!(s.kind, SymbolKind::Func | SymbolKind::GnuIfunc) {
                self.seed(s.value, Seed::Export);
            }
        }
        if elf.entry != 0 {
            self.seed(elf.entry, Seed::Entry);
        }
        for addr in [elf.initializers.init, elf.initializers.fini]
            .into_iter()
            .flatten()
        {
            self.seed(addr, Seed::Initializer);
        }
        for r in &elf.relocations.entries {
            let target = match r.kind {
                R_X86_64_RELATIVE | R_X86_64_IRELATIVE => Some(r.addend as u64),
                R_X86_64_64 => elf
                    .relocation_symbol(r)
                    .filter(|s| s.defined && matches!(s.kind, SymbolKind::Func))
                    .map(|s| s.value.wrapping_add(r.addend as u64)),
                _ => None,
            };
            if let Some(t) = target {
                self.seed(t, Seed::CodePointer);
            }
        }
    }

    /// Whether a call to `target` returns: false for a known noreturn function,
    /// called directly or through its PLT stub.
    fn returns(&mut self, target: u64) -> bool {
        if let Some(&r) = self.callee_cache.get(&target) {
            return r;
        }
        let name = self.names.get(&target).copied().or_else(|| {
            let slot = self.plt_slot(target)?;
            self.names.get(&slot).copied()
        });
        let returns = !name.is_some_and(is_noreturn);
        self.callee_cache.insert(target, returns);
        returns
    }

    /// The GOT slot a PLT stub at `addr` jumps through, if it is one.
    fn plt_slot(&self, addr: u64) -> Option<u64> {
        let (ri, off) = self.locate(addr)?;
        let bytes = &self.regions[ri].bytes[off..];
        let mut decoder = Decoder::with_ip(64, bytes, addr, DecoderOptions::NONE);
        for _ in 0..2 {
            let i = decoder.decode();
            if i.code() == Code::Endbr64 {
                continue;
            }
            let jump = i.flow_control() == FlowControl::IndirectBranch;
            return (jump && i.is_ip_rel_memory_operand()).then(|| i.ip_rel_memory_address());
        }
        None
    }

    /// Follows the trusted seeds, then tries each `lea` target as a tentative
    /// component.
    pub fn run(&mut self) {
        let trusted = std::mem::take(&mut self.work);
        self.component(trusted, false);
        while let Some(t) = self.pending.pop() {
            if self.state(t) == Some(UNKNOWN) {
                self.component(vec![t], true);
            }
        }
    }

    /// Decodes every path from `seeds`. A tentative component that shows any sign
    /// of data is undone as a whole.
    fn component(&mut self, mut work: Vec<u64>, tentative: bool) {
        let seed = work.first().copied().filter(|_| tentative);
        let mut c = Component::default();
        let mut instruction = Instruction::default();
        'paths: while let Some(addr) = work.pop() {
            let Some((ri, off)) = self.locate(addr) else {
                continue;
            };
            match self.regions[ri].state[off] {
                UNKNOWN => {}
                BODY | FDE_BODY if tentative => {
                    c.data = true;
                    break;
                }
                BODY => {
                    c.conflicts += 1;
                    continue;
                }
                _ => continue,
            }
            let bytes = self.regions[ri].bytes;
            let mut decoder = Decoder::with_ip(64, &bytes[off..], addr, DecoderOptions::NONE);
            loop {
                let pos = off + decoder.position();
                if pos >= bytes.len() {
                    break;
                }
                match self.regions[ri].state[pos] {
                    UNKNOWN => {}
                    START | FDE_START => break,
                    _ => {
                        c.conflicts += 1;
                        if tentative {
                            c.data = true;
                            break 'paths;
                        }
                        break;
                    }
                }
                decoder.decode_out(&mut instruction);
                let end = pos + instruction.len();
                let state = &mut self.regions[ri].state;
                let overlaps = end > state.len() || state[pos + 1..end].iter().any(|&s| s != 0);
                if instruction.is_invalid() || overlaps {
                    if instruction.is_invalid() {
                        c.invalid += 1;
                    } else {
                        c.conflicts += 1;
                    }
                    if tentative {
                        c.data = true;
                        break 'paths;
                    }
                    break;
                }
                let kind = access(&instruction);
                if tentative && kind == Some(FsAccess::Unaddressed) {
                    c.data = true;
                    break 'paths;
                }
                state[pos] = START;
                state[pos + 1..end].fill(BODY);
                c.marks.push((ri, pos, end));
                c.bytes += instruction.len() as u64;
                c.instructions += 1;
                if let Some(kind) = kind {
                    let site = prefix_offset(&bytes[pos..end]).map(|p| instruction.ip() + p as u64);
                    c.fs.push((kind, site));
                }
                c.address_taken.extend(address_taken(&instruction));

                let target = direct_target(&instruction);
                match instruction.flow_control() {
                    FlowControl::Next | FlowControl::XbeginXabortXend => {}
                    FlowControl::ConditionalBranch => work.extend(target),
                    FlowControl::UnconditionalBranch => {
                        work.extend(target);
                        break;
                    }
                    FlowControl::Call => {
                        work.extend(target);
                        if target.is_some_and(|t| !self.returns(t)) {
                            c.noreturn_calls += 1;
                            break;
                        }
                    }
                    FlowControl::IndirectCall => {
                        let slot = instruction
                            .is_ip_rel_memory_operand()
                            .then(|| instruction.ip_rel_memory_address());
                        let name = slot.and_then(|s| self.names.get(&s).copied());
                        if name.is_some_and(is_noreturn) {
                            c.noreturn_calls += 1;
                            break;
                        }
                    }
                    FlowControl::Interrupt if instruction.code() != Code::Int3 => {}
                    FlowControl::IndirectBranch
                    | FlowControl::Return
                    | FlowControl::Interrupt
                    | FlowControl::Exception => break,
                }
            }
        }
        self.settle(c, tentative, seed);
    }

    fn settle(&mut self, c: Component, tentative: bool, seed: Option<u64>) {
        if c.data {
            self.out.data_starts.extend(seed);
            for (ri, pos, end) in c.marks {
                self.regions[ri].state[pos..end].fill(UNKNOWN);
            }
            self.out.rejected += 1;
            self.out.rejected_bytes += c.bytes;
            return;
        }
        if tentative {
            *self.out.seeds.entry(Seed::AddressTaken).or_default() += 1;
        }
        let out = &mut self.out;
        out.reached_bytes += c.bytes;
        out.instructions += c.instructions;
        out.noreturn_calls += c.noreturn_calls;
        out.conflicts += c.conflicts;
        out.invalid += c.invalid;
        for (kind, site) in c.fs {
            *out.fs_accesses.entry(kind).or_default() += 1;
            out.rewritable += u64::from(site.is_some());
            out.sites.extend(site);
        }
        self.pending.extend(c.address_taken);
    }

    pub fn into_parts(mut self) -> (Vec<Region<'a>>, Discovery) {
        self.out.data_starts.sort_unstable();
        self.out.data_starts.dedup();
        (self.regions, self.out)
    }
}

/// The address a `lea reg, [rip + disp]` loads.
fn address_taken(i: &Instruction) -> Option<u64> {
    (i.mnemonic() == Mnemonic::Lea && i.is_ip_rel_memory_operand())
        .then(|| i.ip_rel_memory_address())
}

/// The target of a direct near call or jump.
fn direct_target(i: &Instruction) -> Option<u64> {
    (i.op_count() > 0 && i.op0_kind() == OpKind::NearBranch64).then(|| i.near_branch_target())
}
