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
    /// The operating system refused a mapping or protection change.
    Map(io::Error),
    /// Guest code runs only in an x86-64 process (under Rosetta 2 on Apple silicon).
    ForeignHost,
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
            Self::Map(e) => write!(f, "mapping failed: {e}"),
            Self::ForeignHost => write!(f, "guest code runs only in an x86-64 process"),
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
