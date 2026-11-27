use std::collections::HashMap;
use std::fs::{self, File};
use std::os::fd::AsFd;
use std::path::Path;

use cs2metal_elf::consts::{
    EM_X86_64, PF_R, PF_W, PF_X, PT_GNU_RELRO, PT_LOAD, R_X86_64_64, R_X86_64_GLOB_DAT,
    R_X86_64_IRELATIVE, R_X86_64_JUMP_SLOT, R_X86_64_RELATIVE,
};
use cs2metal_elf::{
    DynamicSymbol, Elf, ObjectType, Relocation, Segment, SymbolBinding, SymbolKind,
};

use crate::error::{Error, Result};
use crate::memory::{Region, page_size, round_down, round_up};
use crate::tls::TlsSlot;

const R_X86_64_NONE: u32 = 0;
const R_X86_64_DTPMOD64: u32 = 16;
const R_X86_64_DTPOFF64: u32 = 17;
const R_X86_64_TPOFF64: u32 = 18;
const FS_PREFIX: u8 = 0x64;
const GS_PREFIX: u8 = 0x65;

/// Whether to apply transform T-001, the `fs` to `gs` prefix rewrite, to the code
/// of the modules a namespace maps.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FsRewrite {
    /// Code is mapped as it is on disk.
    #[default]
    Off,
    /// Rewrite every located prefix in proven code; refuse a module with an
    /// unexplained `fs` decode that could be a real access.
    On,
    /// As `On`, and load such a module anyway: the owner's explicit override.
    OnAllowingUnexplained,
}

/// What T-001 did to one module, the counter the transform registry requires.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct FsReport {
    /// Prefix bytes changed from `0x64` to `0x65`.
    pub rewritten: u64,
    /// `fs` decodes in data the code refers to, left alone.
    pub in_data: u64,
    /// Unexplained `fs` decodes that could be real accesses, left alone.
    pub unexplained: u64,
}

/// What a symbol reference binds to.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Bound {
    /// An address, or for a TLS symbol its offset in the defining module's block.
    pub value: u64,
    /// The defining module's TLS slot, for a TLS symbol.
    pub tls: Option<TlsSlot>,
}

impl Bound {
    pub fn address(value: u64) -> Self {
        Self { value, tls: None }
    }
}

/// An import the module asks the caller to resolve.
#[derive(Debug, Clone, Copy)]
pub struct Import<'a> {
    pub name: &'a str,
    /// The symbol version the import requires, if any.
    pub version: Option<&'a str>,
    /// A weak import may stay unresolved; it then binds to address 0.
    pub weak: bool,
}

/// A module mapped into this process.
#[derive(Debug)]
pub struct Image {
    region: Region,
    /// Added to a virtual address of the object to get its address in this process.
    bias: u64,
    /// The lowest page-aligned virtual address, where the region starts.
    lo: u64,
    elf: Elf,
    /// Indices into `elf.symbols` of the defined, non-local symbols, by name.
    exports: HashMap<String, Vec<usize>>,
    /// The module's own TLS slot, if it has a TLS segment in a namespace.
    pub(crate) tls_slot: Option<TlsSlot>,
    /// The virtual address of the program header table in memory.
    phdr: Option<u64>,
    fs_report: Option<FsReport>,
    initialized: bool,
    finalized: bool,
}

fn protection(flags: u32) -> libc::c_int {
    let mut prot = libc::PROT_NONE;
    if flags & PF_R != 0 {
        prot |= libc::PROT_READ;
    }
    if flags & PF_W != 0 {
        prot |= libc::PROT_WRITE;
    }
    if flags & PF_X != 0 {
        prot |= libc::PROT_EXEC;
    }
    prot
}

/// Whether a definition answers a reference to `name` at `version`, by glibc's
/// rules: an unversioned definition answers any reference; a versioned reference
/// needs that version; an unversioned reference takes the default (non-hidden)
/// version.
pub(crate) fn provides(def: &DynamicSymbol, name: &str, version: Option<&str>) -> bool {
    if !def.defined || def.binding == SymbolBinding::Local || def.name != name {
        return false;
    }
    if matches!(def.kind, SymbolKind::Section | SymbolKind::File) {
        return false;
    }
    match (&def.version, version) {
        (None, _) => true,
        (Some(d), Some(v)) => d.name == v,
        (Some(d), None) => !d.hidden,
    }
}

impl Image {
    /// Maps the object at `path` and its segments, all writable for relocation,
    /// and applies T-001 to its code if asked.
    pub(crate) fn map(path: &Path, fs: FsRewrite) -> Result<Self> {
        let data = fs::read(path)?;
        let file = File::open(path)?;
        let elf = cs2metal_elf::parse(&data)?;
        if elf.machine != EM_X86_64 {
            return Err(Error::WrongMachine(elf.machine));
        }
        if elf.object_type != ObjectType::SharedObject {
            return Err(Error::NotRelocatable);
        }
        if elf.relocations.relr_words > 0 {
            return Err(Error::UnsupportedRelr);
        }

        let page = page_size();
        let loads = Self::loads(&elf);
        for s in &loads {
            if s.vaddr % page != s.offset % page {
                return Err(Error::Misaligned {
                    vaddr: s.vaddr,
                    page,
                });
            }
        }
        let lo = loads
            .iter()
            .map(|s| round_down(s.vaddr, page))
            .min()
            .unwrap_or(0);
        let hi = loads
            .iter()
            .map(|s| round_up(s.vaddr + s.memory_size, page))
            .max()
            .unwrap_or(0);
        let region = Region::reserve(hi - lo)?;
        let bias = region.address().wrapping_sub(lo);

        for s in &loads {
            let start = round_down(s.vaddr, page);
            let file_end = s.vaddr + s.file_size;
            let mapped_end = round_up(file_end, page);
            if s.file_size > 0 {
                let file_offset = round_down(s.offset, page);
                region.map_file(start - lo, mapped_end - start, file.as_fd(), file_offset)?;
            }
            let memory_end = s.vaddr + s.memory_size;
            if s.memory_size > s.file_size {
                // .bss: the rest of the last file page, then whole zero pages.
                let tail_end = memory_end.min(mapped_end);
                if s.file_size > 0 && tail_end > file_end {
                    region.zero(file_end - lo, tail_end - file_end)?;
                }
                let zero_start = if s.file_size > 0 { mapped_end } else { start };
                let zero_end = round_up(memory_end, page);
                if zero_end > zero_start {
                    region.map_zero(zero_start - lo, zero_end - zero_start)?;
                }
            }
        }

        let mut exports: HashMap<String, Vec<usize>> = HashMap::new();
        for (i, s) in elf.symbols.iter().enumerate() {
            if s.defined && s.binding != SymbolBinding::Local && !s.name.is_empty() {
                exports.entry(s.name.clone()).or_default().push(i);
            }
        }
        // The program header table: PT_PHDR, or found through the segment that maps
        // the file's first bytes, as the kernel finds it for AT_PHDR.
        let phoff = data
            .get(32..40)
            .map_or(0, |b| u64::from_le_bytes(b.try_into().unwrap_or([0; 8])));
        let phdr = elf
            .segments
            .iter()
            .find(|s| s.kind == cs2metal_elf::consts::PT_PHDR)
            .map(|s| s.vaddr)
            .or_else(|| {
                loads
                    .iter()
                    .find(|s| s.offset == 0)
                    .map(|s| s.vaddr + phoff)
            });
        let mut image = Self {
            region,
            bias,
            lo,
            elf,
            exports,
            tls_slot: None,
            phdr,
            fs_report: None,
            initialized: false,
            finalized: false,
        };
        if fs != FsRewrite::Off {
            image.fs_report = Some(image.rewrite_fs(&data, fs)?);
        }
        Ok(image)
    }

    /// Transform T-001: changes each `fs` prefix that the code scan locates in
    /// proven code to `gs`, in this private mapping only.
    fn rewrite_fs(&self, data: &[u8], policy: FsRewrite) -> Result<FsReport> {
        let scan = cs2metal_codescan::scan_module(data, &self.elf);
        let report = FsReport {
            rewritten: scan.rewrite_sites.len() as u64,
            in_data: scan.data_fs.values().sum(),
            unexplained: scan.unexplained_addressed(),
        };
        if report.unexplained > 0 && policy != FsRewrite::OnAllowingUnexplained {
            return Err(Error::UnexplainedFs(report.unexplained));
        }
        for &site in &scan.rewrite_sites {
            let at = self.offset_span(site, 1)?;
            if self.region.read_u8(at)? != FS_PREFIX {
                return Err(Error::RewriteMismatch { vaddr: site });
            }
            self.region.write_u8(at, GS_PREFIX)?;
        }
        Ok(report)
    }

    /// Maps an object as the kernel maps a program it starts: segments, T-001 if
    /// asked, and segment protections, but no relocation and no RELRO, which a
    /// dynamic loader does for itself.
    pub fn map_program(path: &Path, fs: FsRewrite) -> Result<Self> {
        let image = Self::map(path, fs)?;
        image.protect_segments(false)?;
        Ok(image)
    }

    /// The address in this process of the program header table, for `AT_PHDR`.
    pub fn program_headers(&self) -> Option<u64> {
        self.phdr.map(|v| self.address(v))
    }

    /// What T-001 did to this module, if it ran.
    pub fn fs_report(&self) -> Option<FsReport> {
        self.fs_report
    }

    fn loads(elf: &Elf) -> Vec<Segment> {
        elf.segments
            .iter()
            .filter(|s| s.kind == PT_LOAD)
            .cloned()
            .collect()
    }

    /// Sets the final segment protections and makes RELRO read-only.
    pub(crate) fn protect(&self) -> Result<()> {
        self.protect_segments(true)
    }

    /// Sets each segment's protection; RELRO too if asked. A program the kernel
    /// starts gets no RELRO: its dynamic loader relocates first, then protects.
    fn protect_segments(&self, relro: bool) -> Result<()> {
        let page = page_size();
        for s in Self::loads(&self.elf) {
            let start = round_down(s.vaddr, page);
            let end = round_up(s.vaddr + s.memory_size, page);
            self.region
                .protect(start - self.lo, end - start, protection(s.flags))?;
        }
        let relro_segment = self
            .elf
            .segments
            .iter()
            .find(|s| s.kind == PT_GNU_RELRO)
            .filter(|_| relro);
        if let Some(relro) = relro_segment {
            // glibc rounds the end of RELRO down: the page holding its tail may
            // also hold writable data.
            let start = round_down(relro.vaddr, page);
            let end = round_down(relro.vaddr + relro.memory_size, page);
            if end > start {
                self.region
                    .protect(start - self.lo, end - start, libc::PROT_READ)?;
            }
        }
        Ok(())
    }

    /// Maps the object at `path` on its own, relocates it, and protects its
    /// segments. A symbol it defines binds to itself; `resolve` answers each import
    /// it does not define, with an address or `None`. Initializers have not run.
    /// [`Namespace`](crate::Namespace) loads modules together, with a global scope.
    pub fn load(path: &Path, resolve: &mut dyn FnMut(Import<'_>) -> Option<u64>) -> Result<Self> {
        let image = Self::map(path, FsRewrite::Off)?;
        image.relocate(&mut |symbol| {
            if symbol.defined {
                return image.bound(symbol);
            }
            let import = Import {
                name: &symbol.name,
                version: symbol.version.as_ref().map(|v| v.name.as_str()),
                weak: symbol.binding == SymbolBinding::Weak,
            };
            match resolve(import) {
                Some(address) => Ok(Bound::address(address)),
                None if import.weak => Ok(Bound::address(0)),
                None => Err(Error::Unresolved(symbol.name.clone())),
            }
        })?;
        image.protect()?;
        Ok(image)
    }

    /// The address in this process of an object virtual address.
    pub fn address(&self, vaddr: u64) -> u64 {
        self.bias.wrapping_add(vaddr)
    }

    pub fn elf(&self) -> &Elf {
        &self.elf
    }

    /// The name other modules need this one by: its `DT_SONAME`, if it has one.
    pub fn soname(&self) -> Option<&str> {
        self.elf.soname.as_deref()
    }

    /// The definition this module gives for a reference, if any.
    pub(crate) fn find(&self, name: &str, version: Option<&str>) -> Option<&DynamicSymbol> {
        self.exports
            .get(name)?
            .iter()
            .map(|&i| &self.elf.symbols[i])
            .find(|s| provides(s, name, version))
    }

    /// What a reference to one of this module's definitions binds to: an address,
    /// or for a TLS symbol its offset and this module's slot.
    pub(crate) fn bound(&self, symbol: &DynamicSymbol) -> Result<Bound> {
        if symbol.kind == SymbolKind::Tls {
            let tls = self.tls_slot.ok_or(Error::UnsupportedTls)?;
            return Ok(Bound {
                value: symbol.value,
                tls: Some(tls),
            });
        }
        self.definition(symbol).map(Bound::address)
    }

    /// The run-time address of one of this module's definitions. An IFUNC is
    /// resolved by calling its resolver.
    pub(crate) fn definition(&self, symbol: &DynamicSymbol) -> Result<u64> {
        let address = self.address(symbol.value);
        if symbol.kind == SymbolKind::GnuIfunc {
            // SAFETY: the resolver is relocated code of this module; modules are
            // relocated dependencies first, so what it calls is in place.
            return unsafe { call_resolver(address) };
        }
        Ok(address)
    }

    /// The address of a symbol the module defines and exports, at its default
    /// version.
    pub fn symbol(&self, name: &str) -> Option<u64> {
        self.find(name, None)
            .filter(|s| s.kind != SymbolKind::Tls)
            .and_then(|s| self.definition(s).ok())
    }

    /// Reads the 64-bit value at an object virtual address inside a loaded segment.
    pub fn read_u64(&self, vaddr: u64) -> Result<u64> {
        self.region.read_u64(self.offset(vaddr)?)
    }

    /// The offset in the region of an object virtual address, checked to lie in a
    /// loaded segment.
    fn offset(&self, vaddr: u64) -> Result<u64> {
        self.offset_span(vaddr, 8)
    }

    fn offset_span(&self, vaddr: u64, len: u64) -> Result<u64> {
        let inside = self.elf.segments.iter().any(|s| {
            s.kind == PT_LOAD
                && vaddr >= s.vaddr
                && vaddr.saturating_add(len) <= s.vaddr + s.memory_size
        });
        if !inside {
            return Err(Error::RelocationOutOfBounds { offset: vaddr });
        }
        Ok(vaddr - self.lo)
    }

    /// Applies every relocation. `bind` gives the address a symbol reference binds
    /// to; it is called for the references this module makes, including to symbols
    /// it defines itself, which another module may interpose.
    pub(crate) fn relocate(
        &self,
        bind: &mut dyn FnMut(&DynamicSymbol) -> Result<Bound>,
    ) -> Result<()> {
        // IRELATIVE entries run guest resolvers, which may read other relocated
        // data, so they go last, as in glibc.
        let (deferred, first): (Vec<&Relocation>, Vec<&Relocation>) = self
            .elf
            .relocations
            .entries
            .iter()
            .partition(|r| r.kind == R_X86_64_IRELATIVE);
        for r in first.into_iter().chain(deferred) {
            let addend = r.addend as u64;
            let value = match r.kind {
                R_X86_64_NONE => continue,
                R_X86_64_RELATIVE => self.address(addend),
                R_X86_64_IRELATIVE => {
                    // SAFETY: the module's own resolver, after every other
                    // relocation of the module is in place.
                    unsafe { call_resolver(self.address(addend))? }
                }
                R_X86_64_GLOB_DAT | R_X86_64_JUMP_SLOT | R_X86_64_64 => {
                    let symbol = self
                        .elf
                        .relocation_symbol(r)
                        .ok_or(Error::RelocationOutOfBounds { offset: r.offset })?;
                    let target = if symbol.binding == SymbolBinding::Local {
                        self.definition(symbol)?
                    } else {
                        bind(symbol)?.value
                    };
                    if r.kind == R_X86_64_64 {
                        target.wrapping_add(addend)
                    } else {
                        target
                    }
                }
                R_X86_64_DTPMOD64 | R_X86_64_DTPOFF64 | R_X86_64_TPOFF64 => {
                    self.tls_value(r, addend, bind)?
                }
                other => return Err(Error::UnsupportedRelocation(other)),
            };
            self.region.write_u64(self.offset(r.offset)?, value)?;
        }
        Ok(())
    }

    /// The value of a TLS relocation. Symbol 0 means this module's own block (the
    /// local-dynamic model and static TLS of local variables).
    fn tls_value(
        &self,
        r: &Relocation,
        addend: u64,
        bind: &mut dyn FnMut(&DynamicSymbol) -> Result<Bound>,
    ) -> Result<u64> {
        let (value, slot) = match self.elf.relocation_symbol(r) {
            None => (0, self.tls_slot),
            Some(symbol) if symbol.binding == SymbolBinding::Local => (symbol.value, self.tls_slot),
            Some(symbol) => {
                let bound = bind(symbol)?;
                (bound.value, bound.tls)
            }
        };
        let slot = slot.ok_or(Error::UnsupportedTls)?;
        Ok(match r.kind {
            R_X86_64_DTPMOD64 => slot.id,
            R_X86_64_DTPOFF64 => value.wrapping_add(addend),
            // The block is at tp - offset, so the variable is at tp + this.
            _ => value.wrapping_add(addend).wrapping_sub(slot.offset),
        })
    }

    fn array(&self, at: Option<u64>, entries: u64) -> Result<Vec<u64>> {
        let Some(at) = at else {
            return Ok(Vec::new());
        };
        (0..entries).map(|i| self.read_u64(at + 8 * i)).collect()
    }

    /// Runs `DT_INIT`, then `DT_INIT_ARRAY` in order, once.
    ///
    /// # Safety
    ///
    /// Runs guest code. The process must be x86-64, the module's imports must be
    /// bound to code that honours their contracts, and nothing the module needs at
    /// initialization (such as TLS or a C library) may be missing.
    pub unsafe fn run_initializers(&mut self) -> Result<()> {
        if !cfg!(target_arch = "x86_64") {
            return Err(Error::ForeignHost);
        }
        if self.initialized {
            return Ok(());
        }
        self.initialized = true;
        let init = self.elf.initializers;
        let array = self.array(init.init_array, init.init_array_entries)?;
        let entries = init.init.map(|v| self.address(v)).into_iter().chain(array);
        for f in entries.filter(|&f| f != 0 && f != u64::MAX) {
            // SAFETY: the caller upholds this function's contract; glibc calls
            // initializers with argc, argv, and envp.
            unsafe { call_initializer(f) };
        }
        Ok(())
    }

    /// Runs `DT_FINI_ARRAY` in reverse, then `DT_FINI`, once, if initializers ran.
    ///
    /// # Safety
    ///
    /// As for [`Image::run_initializers`].
    pub unsafe fn run_finalizers(&mut self) -> Result<()> {
        if !self.initialized || self.finalized {
            return Ok(());
        }
        self.finalized = true;
        let fini = self.elf.initializers;
        let mut array = self.array(fini.fini_array, fini.fini_array_entries)?;
        array.reverse();
        let entries = array.into_iter().chain(fini.fini.map(|v| self.address(v)));
        for f in entries.filter(|&f| f != 0 && f != u64::MAX) {
            // SAFETY: as in run_initializers.
            unsafe { call_initializer(f) };
        }
        Ok(())
    }
}

/// Calls an IFUNC resolver at `address` and returns the implementation it picks.
///
/// # Safety
///
/// `address` must be a relocated IFUNC resolver of a loaded module, and the process
/// must be x86-64.
#[cfg(target_arch = "x86_64")]
unsafe fn call_resolver(address: u64) -> Result<u64> {
    // SAFETY: the caller guarantees address is a resolver taking no arguments and
    // returning a code address, which is glibc's contract for them.
    let resolver: extern "C" fn() -> u64 = unsafe { std::mem::transmute(address as usize) };
    Ok(resolver())
}

#[cfg(not(target_arch = "x86_64"))]
unsafe fn call_resolver(_address: u64) -> Result<u64> {
    Err(Error::ForeignHost)
}

/// Calls an initializer or finalizer as glibc does, with no arguments of interest.
///
/// # Safety
///
/// `address` must be a relocated initializer of a loaded module.
#[cfg(target_arch = "x86_64")]
unsafe fn call_initializer(address: u64) {
    type Init = extern "C" fn(i32, *const *const u8, *const *const u8);
    // SAFETY: the caller guarantees address is an initializer; glibc passes argc,
    // argv, and envp, which initializers may ignore.
    let f: Init = unsafe { std::mem::transmute(address as usize) };
    f(0, std::ptr::null(), std::ptr::null());
}

#[cfg(not(target_arch = "x86_64"))]
unsafe fn call_initializer(_address: u64) {}
