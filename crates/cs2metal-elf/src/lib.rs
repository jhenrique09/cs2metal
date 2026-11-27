//! Read-only parsing of little-endian ELF64 objects, as the CS2 Linux build ships them.
//!
//! The parser reads what a dynamic loader reads: the program headers and the dynamic
//! segment. Section headers are consulted only to size the dynamic symbol table when
//! an object has neither `DT_HASH` nor `DT_GNU_HASH`. Every read is bounds-checked
//! and malformed input yields an [`Error`], never a panic.
//!
//! ```
//! # fn demo(bytes: &[u8]) -> Result<(), cs2metal_elf::Error> {
//! let elf = cs2metal_elf::parse(bytes)?;
//! for import in elf.imports() {
//!     println!("{} needs {}", elf.soname.as_deref().unwrap_or("?"), import.name);
//! }
//! # Ok(()) }
//! ```

pub mod consts;
mod eh_frame;
mod error;
mod parse;
mod reader;
#[cfg(any(test, feature = "test-support"))]
pub mod testing;
#[cfg(test)]
mod tests;

use std::collections::BTreeMap;

use serde::Serialize;

pub use eh_frame::{FunctionRange, unwind_ranges};
pub use error::{Error, Result};
pub use parse::parse;

/// The `e_type` of an object.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ObjectType {
    Relocatable,
    /// A position-dependent executable (`ET_EXEC`), linked at a fixed address.
    Executable,
    /// A shared object or a position-independent executable (`ET_DYN`).
    SharedObject,
    Core,
    Other(u16),
}

/// One program header.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Segment {
    pub kind: u32,
    pub flags: u32,
    pub offset: u64,
    pub vaddr: u64,
    pub file_size: u64,
    pub memory_size: u64,
    pub align: u64,
}

impl Segment {
    pub fn name(&self) -> &'static str {
        consts::segment_name(self.kind)
    }

    pub fn readable(&self) -> bool {
        self.flags & consts::PF_R != 0
    }

    pub fn writable(&self) -> bool {
        self.flags & consts::PF_W != 0
    }

    pub fn executable(&self) -> bool {
        self.flags & consts::PF_X != 0
    }

    /// `rwx`-style permissions, as `readelf` prints them.
    pub fn permissions(&self) -> String {
        let flag = |on: bool, c: char| if on { c } else { '-' };
        [
            flag(self.readable(), 'r'),
            flag(self.writable(), 'w'),
            flag(self.executable(), 'x'),
        ]
        .iter()
        .collect()
    }
}

/// The `PT_TLS` template: the initialized image and the size of each thread's block.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct TlsTemplate {
    pub vaddr: u64,
    pub file_size: u64,
    pub memory_size: u64,
    pub align: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SymbolKind {
    NoType,
    Object,
    Func,
    Section,
    File,
    Common,
    Tls,
    /// An indirect function: the loader calls a resolver to choose the implementation.
    GnuIfunc,
    Other(u8),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SymbolBinding {
    Local,
    Global,
    Weak,
    GnuUnique,
    Other(u8),
}

/// The GNU version attached to a dynamic symbol.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub struct SymbolVersion {
    /// The version name, such as `GLIBC_2.2.5`.
    pub name: String,
    /// For an import, the library the version requirement names (`libc.so.6`).
    pub library: Option<String>,
    /// A hidden (non-default) version, written `name@VERSION` instead of `@@`.
    pub hidden: bool,
}

/// One entry of the dynamic symbol table (index 0 excluded).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DynamicSymbol {
    pub name: String,
    pub kind: SymbolKind,
    pub binding: SymbolBinding,
    /// False for an undefined symbol (`st_shndx == SHN_UNDEF`), which is an import.
    pub defined: bool,
    pub value: u64,
    pub size: u64,
    pub version: Option<SymbolVersion>,
}

impl DynamicSymbol {
    /// An undefined, non-local symbol that the loader must bind.
    pub fn is_import(&self) -> bool {
        !self.defined && self.binding != SymbolBinding::Local && !self.name.is_empty()
    }

    /// A defined, non-local symbol other modules may bind to.
    pub fn is_export(&self) -> bool {
        self.defined && self.binding != SymbolBinding::Local && !self.name.is_empty()
    }
}

/// A `DT_VERNEED` entry: the versions this object requires from one library.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct VersionNeed {
    pub library: String,
    pub versions: Vec<String>,
}

/// What the relocation tables ask the loader to do.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Relocations {
    /// Count per x86-64 relocation type, over `DT_RELA` and `DT_JMPREL`.
    pub counts: BTreeMap<u32, u64>,
    /// Words in the `DT_RELR` packed relative relocation table.
    pub relr_words: u64,
    /// Symbols named by `R_X86_64_COPY` relocations, which copy a library's data
    /// object into the executable.
    pub copy_symbols: Vec<String>,
}

impl Relocations {
    pub fn total(&self) -> u64 {
        self.counts.values().sum()
    }
}

/// Initialization and finalization entry points.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct Initializers {
    pub init: Option<u64>,
    pub fini: Option<u64>,
    pub init_array_entries: u64,
    pub fini_array_entries: u64,
    pub preinit_array_entries: u64,
}

/// Dynamic flags that change how the loader binds or lays out the object.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct DynamicFlags {
    pub bind_now: bool,
    pub text_relocations: bool,
    /// The object uses the initial-exec TLS model and needs static TLS space.
    pub static_tls: bool,
    pub symbolic: bool,
}

/// A parsed ELF object.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Elf {
    pub object_type: ObjectType,
    pub machine: u16,
    pub entry: u64,
    pub segments: Vec<Segment>,
    pub interpreter: Option<String>,
    /// False for an object with no `PT_DYNAMIC`, such as a static executable.
    pub dynamic: bool,
    pub soname: Option<String>,
    pub needed: Vec<String>,
    pub rpath: Vec<String>,
    pub runpath: Vec<String>,
    pub tls: Option<TlsTemplate>,
    pub symbols: Vec<DynamicSymbol>,
    pub version_needs: Vec<VersionNeed>,
    pub version_definitions: Vec<String>,
    pub relocations: Relocations,
    pub initializers: Initializers,
    pub flags: DynamicFlags,
}

impl Elf {
    pub fn machine_name(&self) -> &'static str {
        consts::machine_name(self.machine)
    }

    /// A position-independent executable: `ET_DYN` with a program interpreter.
    pub fn is_pie(&self) -> bool {
        self.object_type == ObjectType::SharedObject && self.interpreter.is_some()
    }

    pub fn imports(&self) -> impl Iterator<Item = &DynamicSymbol> {
        self.symbols.iter().filter(|s| s.is_import())
    }

    pub fn exports(&self) -> impl Iterator<Item = &DynamicSymbol> {
        self.symbols.iter().filter(|s| s.is_export())
    }

    /// The span of virtual addresses the `PT_LOAD` segments occupy, page-aligned.
    pub fn load_span(&self) -> Option<(u64, u64)> {
        const PAGE: u64 = 0x1000;
        let loads = self.segments.iter().filter(|s| s.kind == consts::PT_LOAD);
        let (lo, hi) = loads.fold((u64::MAX, 0u64), |(lo, hi), s| {
            (
                lo.min(s.vaddr),
                hi.max(s.vaddr.saturating_add(s.memory_size)),
            )
        });
        (lo < hi).then(|| (lo & !(PAGE - 1), hi.saturating_add(PAGE - 1) & !(PAGE - 1)))
    }
}
