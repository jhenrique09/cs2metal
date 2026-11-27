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

const R_X86_64_NONE: u32 = 0;

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

impl Image {
    /// Maps the object at `path`, applies its relocations, and protects its
    /// segments. `resolve` answers each import the object does not define itself;
    /// it returns the import's address, or `None`. Initializers have not run yet.
    pub fn load(path: &Path, resolve: &mut dyn FnMut(Import<'_>) -> Option<u64>) -> Result<Self> {
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
        let loads: Vec<Segment> = elf
            .segments
            .iter()
            .filter(|s| s.kind == PT_LOAD)
            .cloned()
            .collect();
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

        // Map every segment writable first; protections are set after relocation.
        for s in &loads {
            let start = round_down(s.vaddr, page);
            let file_end = s.vaddr + s.file_size;
            let mapped_end = round_up(file_end, page);
            if s.file_size > 0 {
                region.map_file(
                    start - lo,
                    mapped_end - start,
                    file.as_fd(),
                    round_down(s.offset, page),
                )?;
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

        let image = Self {
            region,
            bias,
            lo,
            elf,
            initialized: false,
            finalized: false,
        };
        image.relocate(resolve)?;

        for s in &loads {
            let start = round_down(s.vaddr, page);
            let end = round_up(s.vaddr + s.memory_size, page);
            image
                .region
                .protect(start - lo, end - start, protection(s.flags))?;
        }
        if let Some(relro) = image.elf.segments.iter().find(|s| s.kind == PT_GNU_RELRO) {
            // glibc rounds the end of RELRO down: the page holding its tail may
            // also hold writable data.
            let start = round_down(relro.vaddr, page);
            let end = round_down(relro.vaddr + relro.memory_size, page);
            if end > start {
                image
                    .region
                    .protect(start - lo, end - start, libc::PROT_READ)?;
            }
        }
        Ok(image)
    }

    /// The address in this process of an object virtual address.
    pub fn address(&self, vaddr: u64) -> u64 {
        self.bias.wrapping_add(vaddr)
    }

    pub fn elf(&self) -> &Elf {
        &self.elf
    }

    /// The address of a symbol the module defines and exports.
    pub fn symbol(&self, name: &str) -> Option<u64> {
        self.elf
            .exports()
            .find(|s| s.name == name && s.kind != SymbolKind::Tls)
            .map(|s| self.address(s.value))
    }

    /// The offset in the region of an object virtual address, checked to lie in a
    /// loaded segment.
    fn offset(&self, vaddr: u64) -> Result<u64> {
        let inside = self.elf.segments.iter().any(|s| {
            s.kind == PT_LOAD
                && vaddr >= s.vaddr
                && vaddr.saturating_add(8) <= s.vaddr + s.memory_size
        });
        if !inside {
            return Err(Error::RelocationOutOfBounds { offset: vaddr });
        }
        Ok(vaddr - self.lo)
    }

    fn bind(
        &self,
        symbol: &DynamicSymbol,
        resolve: &mut dyn FnMut(Import<'_>) -> Option<u64>,
    ) -> Result<u64> {
        if symbol.defined {
            let address = self.address(symbol.value);
            if symbol.kind == SymbolKind::GnuIfunc {
                // SAFETY: the resolver is the module's own code, already relocated
                // up to this point, called as glibc calls it.
                return unsafe { call_resolver(address) };
            }
            return Ok(address);
        }
        let weak = symbol.binding == SymbolBinding::Weak;
        let import = Import {
            name: &symbol.name,
            version: symbol.version.as_ref().map(|v| v.name.as_str()),
            weak,
        };
        match resolve(import) {
            Some(address) => Ok(address),
            None if weak => Ok(0),
            None => Err(Error::Unresolved(symbol.name.clone())),
        }
    }

    fn relocate(&self, resolve: &mut dyn FnMut(Import<'_>) -> Option<u64>) -> Result<()> {
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
                    // SAFETY: as in bind: the module's own resolver, after every other
                    // relocation is in place.
                    unsafe { call_resolver(self.address(addend))? }
                }
                R_X86_64_GLOB_DAT | R_X86_64_JUMP_SLOT | R_X86_64_64 => {
                    let symbol = self
                        .elf
                        .relocation_symbol(r)
                        .ok_or(Error::RelocationOutOfBounds { offset: r.offset })?;
                    let target = self.bind(symbol, resolve)?;
                    if r.kind == R_X86_64_64 {
                        target.wrapping_add(addend)
                    } else {
                        target
                    }
                }
                other => return Err(Error::UnsupportedRelocation(other)),
            };
            self.region.write_u64(self.offset(r.offset)?, value)?;
        }
        Ok(())
    }

    fn array(&self, at: Option<u64>, entries: u64) -> Result<Vec<u64>> {
        let Some(at) = at else {
            return Ok(Vec::new());
        };
        (0..entries)
            .map(|i| self.region.read_u64(self.offset(at + 8 * i)?))
            .collect()
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
