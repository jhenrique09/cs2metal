use std::fmt;

/// Why an object could not be parsed. Every malformed input maps to one of these;
/// the parser never panics.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// The file does not start with the ELF magic.
    NotElf,
    /// Only ELF64 is supported.
    UnsupportedClass(u8),
    /// Only little-endian objects are supported.
    UnsupportedEncoding(u8),
    /// `e_ident[EI_VERSION]` is not `EV_CURRENT`.
    UnsupportedVersion(u8),
    /// A read of `len` bytes at file offset `offset` falls outside the file.
    OutOfBounds { offset: u64, len: u64 },
    /// A virtual address is not backed by file data in any `PT_LOAD` segment.
    UnmappedAddress(u64),
    /// A table declares an entry size the parser does not understand.
    BadEntrySize { table: &'static str, size: u64 },
    /// A string table offset does not name a NUL-terminated string.
    BadString { offset: u64 },
    /// The dynamic symbol table cannot be sized: no `DT_HASH`, no `DT_GNU_HASH`,
    /// and no `SHT_DYNSYM` section header.
    UnknownSymbolCount,
    /// The object uses `DT_REL` relocations, which x86-64 objects do not use.
    UnsupportedRelocationFormat(u64),
    /// A dynamic table address appears without the size that goes with it.
    MissingTag(&'static str),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotElf => write!(f, "not an ELF file"),
            Self::UnsupportedClass(c) => write!(f, "unsupported ELF class {c} (only ELF64)"),
            Self::UnsupportedEncoding(d) => {
                write!(f, "unsupported data encoding {d} (only little-endian)")
            }
            Self::UnsupportedVersion(v) => write!(f, "unsupported ELF version {v}"),
            Self::OutOfBounds { offset, len } => {
                write!(
                    f,
                    "read of {len} bytes at offset {offset:#x} is outside the file"
                )
            }
            Self::UnmappedAddress(a) => write!(f, "address {a:#x} is not backed by file data"),
            Self::BadEntrySize { table, size } => {
                write!(f, "unexpected {table} entry size {size}")
            }
            Self::BadString { offset } => write!(f, "bad string table offset {offset:#x}"),
            Self::UnknownSymbolCount => write!(f, "cannot size the dynamic symbol table"),
            Self::UnsupportedRelocationFormat(t) => {
                write!(f, "unsupported relocation table format (tag {t})")
            }
            Self::MissingTag(tag) => write!(f, "missing dynamic tag {tag}"),
        }
    }
}

impl std::error::Error for Error {}

pub type Result<T> = std::result::Result<T, Error>;
