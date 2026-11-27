use std::fmt;
use std::io;

/// Why a module could not be loaded.
#[derive(Debug)]
pub enum Error {
    Io(io::Error),
    Elf(cs2metal_elf::Error),
    /// Only x86-64 objects run here.
    WrongMachine(u16),
    /// Only shared objects and position-independent executables can be placed.
    NotRelocatable,
    /// A segment is not aligned to the host page size, so it cannot be mapped.
    Misaligned {
        vaddr: u64,
        page: u64,
    },
    /// A relocation type this loader does not implement yet.
    UnsupportedRelocation(u32),
    /// The object uses `DT_RELR`, not implemented yet.
    UnsupportedRelr,
    /// A relocation writes outside the module's writable memory.
    RelocationOutOfBounds {
        offset: u64,
    },
    /// A non-weak import that nothing provides.
    Unresolved(String),
    /// The same, for a module loaded in a namespace.
    UnresolvedIn {
        name: String,
        module: String,
    },
    /// A needed library found in no search directory.
    NotFound {
        name: String,
        needed_by: String,
    },
    /// The operating system refused a mapping or protection change.
    Map(io::Error),
    /// Guest code runs only in an x86-64 process (under Rosetta 2 on Apple silicon).
    ForeignHost,
    /// A TLS relocation or symbol outside a namespace's TLS layout.
    UnsupportedTls,
    /// T-001 found this many unexplained `fs` decodes that could be real accesses.
    UnexplainedFs(u64),
    /// A byte T-001 was to rewrite is not an `fs` prefix: the scan and the
    /// mapping disagree.
    RewriteMismatch {
        vaddr: u64,
    },
    /// Guest calls take at most six integer arguments.
    TooManyArguments(usize),
}

pub type Result<T> = std::result::Result<T, Error>;

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(e) => write!(f, "{e}"),
            Self::Elf(e) => write!(f, "{e}"),
            Self::WrongMachine(m) => write!(f, "machine {m} is not x86-64"),
            Self::NotRelocatable => write!(f, "not a shared object or a PIE"),
            Self::Misaligned { vaddr, page } => {
                write!(
                    f,
                    "segment at {vaddr:#x} is not aligned to the {page}-byte page"
                )
            }
            Self::UnsupportedRelocation(t) => write!(f, "unsupported relocation type {t}"),
            Self::UnsupportedRelr => write!(f, "DT_RELR relocations are not supported yet"),
            Self::RelocationOutOfBounds { offset } => {
                write!(f, "relocation at {offset:#x} is outside writable memory")
            }
            Self::Unresolved(name) => write!(f, "unresolved import {name}"),
            Self::UnresolvedIn { name, module } => {
                write!(f, "unresolved import {name} of {module}")
            }
            Self::NotFound { name, needed_by } => {
                write!(f, "{name}, needed by {needed_by}, was not found")
            }
            Self::Map(e) => write!(f, "mapping failed: {e}"),
            Self::ForeignHost => write!(f, "guest code runs only in an x86-64 process"),
            Self::UnsupportedTls => write!(f, "TLS outside a namespace's TLS layout"),
            Self::UnexplainedFs(n) => write!(
                f,
                "{n} unexplained fs decodes could be real accesses; refusing T-001"
            ),
            Self::RewriteMismatch { vaddr } => {
                write!(
                    f,
                    "byte at {vaddr:#x} is not the fs prefix the scan located"
                )
            }
            Self::TooManyArguments(n) => write!(f, "{n} arguments; guest calls take at most 6"),
        }
    }
}

impl std::error::Error for Error {}

impl From<io::Error> for Error {
    fn from(e: io::Error) -> Self {
        Self::Io(e)
    }
}

impl From<cs2metal_elf::Error> for Error {
    fn from(e: cs2metal_elf::Error) -> Self {
        Self::Elf(e)
    }
}
