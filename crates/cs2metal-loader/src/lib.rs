//! Maps a Linux x86-64 ELF shared object into this process, relocates it, and runs
//! its initializers, the way glibc's `ld.so` would, for the subset the CS2 build
//! needs.
//!
//! This is the first slice of roadmap step C2: one module, its own symbols, and an
//! import resolver supplied by the caller. Symbol scopes across modules, TLS, the
//! GS bridge, and transform T-001 come next.
//!
//! The file is opened read-only and mapped privately: writes made while relocating
//! land in copy-on-write pages and never reach the disk. Guest code only runs in an
//! x86-64 process, which on Apple silicon means under Rosetta 2.

mod error;
mod image;
mod memory;

pub use error::{Error, Result};
pub use image::{Image, Import};
