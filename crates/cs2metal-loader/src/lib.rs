//! Maps a Linux x86-64 ELF shared object into this process, relocates it, and runs
//! its initializers, the way glibc's `ld.so` would, for the subset the CS2 build
//! needs.
//!
//! [`Image`] is one module; [`Namespace`] loads modules with their dependencies into
//! one global scope, with glibc's lookup rules: symbol versions, first definition
//! in load order wins (so a later module's definitions can be interposed), weak
//! references may stay unresolved. Imports no module defines go to a host
//! resolver.
//!
//! Thread-local storage follows glibc's variant II layout with every block static
//! ([`GuestThread`]). Guest code reaches its control block through `gs` once
//! transform T-001 has rewritten its `fs` prefixes ([`FsRewrite`]), and the
//! [`Bridge`] switches the GS base between guest and host domains.
//!
//! The file is opened read-only and mapped privately: writes made while relocating
//! land in copy-on-write pages and never reach the disk. Guest code only runs in an
//! x86-64 process, which on Apple silicon means under Rosetta 2.

mod bridge;
mod error;
mod image;
mod memory;
mod namespace;
mod tls;

pub use bridge::{
    Bridge, HostBlock, PROXY_SLOTS, ReturnFixup, SyscallFrame, SyscallHandler, attach,
    enter_process, leave_guest_thread, neutral, run_guest_thread, set_return_fixup,
    set_syscall_handler, syscall_entry,
};
pub use error::{Error, Result};
pub use image::{FsReport, FsRewrite, Image, Import};
pub use namespace::{Module, Namespace};
pub use tls::{GuestThread, TlsSlot};
