//! Maps a Linux x86-64 ELF shared object into this process, relocates it, and runs
//! its initializers, the way glibc's `ld.so` would, for the subset the CS2 build
//! needs.
//!
//! [`Image`] is one module; [`Namespace`] loads modules with their dependencies into
//! one global scope, with glibc's lookup rules: symbol versions, first definition
//! in load order wins (so a later module's definitions can be interposed), weak
//! references may stay unresolved. Imports no module defines go to a host
//! resolver. TLS, the GS bridge, and transform T-001 come next.
//!
//! The file is opened read-only and mapped privately: writes made while relocating
//! land in copy-on-write pages and never reach the disk. Guest code only runs in an
//! x86-64 process, which on Apple silicon means under Rosetta 2.

mod error;
mod image;
mod memory;
mod namespace;

pub use error::{Error, Result};
pub use image::{Image, Import};
pub use namespace::{Module, Namespace};
