//! Read-only scan of guest x86-64 code for what the `fs` to `gs` rewrite (transform
//! T-001) and the TLS layer need to know.
//!
//! For each module the scan decodes every instruction inside the code ranges the
//! `.eh_frame` unwind tables describe, classifies each `fs`-prefixed instruction,
//! and checks that its `0x64` prefix byte can be located for a same-length rewrite.
//! Code outside those ranges is swept linearly only to count `fs` prefixes there:
//! they are what T-001 could not rewrite safely. Code means the executable sections
//! when the object has a section table, because some linkers map read-only data in
//! the executable segment, and the executable segments otherwise. The TLS models a
//! module uses come from its dynamic relocations and from its code.
//!
//! Nothing is modified. Reports derived from Valve's binaries stay local.

mod report;
mod scan;
mod walk;

use std::collections::BTreeMap;

use serde::Serialize;

pub use report::markdown;
pub use scan::scan_module;
pub use walk::scan_path;

/// The kind of an `fs`-prefixed memory access.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FsAccess {
    /// `fs:0`: loads the thread pointer, the start of most TLS sequences.
    ThreadPointer,
    /// `fs:0x28`: glibc's stack-protector canary.
    StackGuard,
    /// `fs:0x30`: glibc's pointer guard.
    PointerGuard,
    /// Any other constant offset: a local-exec TLS variable or another control
    /// block field.
    ConstantOffset,
    /// An offset held in a register: initial-exec TLS, after a GOT load.
    RegisterRelative,
}

impl FsAccess {
    pub fn label(self) -> &'static str {
        match self {
            Self::ThreadPointer => "fs:0",
            Self::StackGuard => "fs:0x28",
            Self::PointerGuard => "fs:0x30",
            Self::ConstantOffset => "fs:constant",
            Self::RegisterRelative => "fs:register",
        }
    }

    pub const ALL: [Self; 5] = [
        Self::ThreadPointer,
        Self::StackGuard,
        Self::PointerGuard,
        Self::ConstantOffset,
        Self::RegisterRelative,
    ];
}

/// TLS models a module uses, as counts of the evidence for each.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct TlsModels {
    /// `R_X86_64_DTPMOD64`: global- or local-dynamic, through `__tls_get_addr`.
    pub dynamic_module_relocations: u64,
    /// `R_X86_64_TLSDESC`: TLS descriptors.
    pub descriptor_relocations: u64,
    /// `R_X86_64_TPOFF64`: initial-exec resolved by the loader.
    pub initial_exec_relocations: u64,
    /// `PT_TLS` size of the module's own TLS block, in bytes.
    pub tls_block_bytes: u64,
    /// `DF_STATIC_TLS`: the module needs static TLS space.
    pub static_tls_flag: bool,
}

impl TlsModels {
    pub fn summary(&self, code: &BTreeMap<FsAccess, u64>) -> Vec<&'static str> {
        let mut models = Vec::new();
        if self.dynamic_module_relocations > 0 {
            models.push("dynamic");
        }
        if self.descriptor_relocations > 0 {
            models.push("descriptors");
        }
        if self.initial_exec_relocations > 0
            || code.get(&FsAccess::RegisterRelative).copied().unwrap_or(0) > 0
        {
            models.push("initial-exec");
        }
        if code.get(&FsAccess::ConstantOffset).copied().unwrap_or(0) > 0 {
            models.push("local-exec");
        }
        models
    }
}

/// The scan of one module.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct ModuleScan {
    pub machine: String,
    /// Whether code was bounded by the section table rather than by segments.
    pub code_from_sections: bool,
    /// Why the section table could not be read, if it could not. The scan then
    /// falls back to whole segments.
    pub section_error: Option<String>,
    /// Bytes of code: executable sections inside executable `PT_LOAD` segments, or
    /// the segments' file bytes when there is no section table.
    pub code_bytes: u64,
    /// Code bytes inside some FDE range.
    pub covered_bytes: u64,
    pub functions: usize,
    /// Why the unwind tables could not be read, if they could not.
    pub unwind_error: Option<String>,
    /// `fs` accesses inside covered ranges, by kind.
    pub fs_accesses: BTreeMap<FsAccess, u64>,
    /// Covered `fs` accesses whose `0x64` prefix byte was located, so a
    /// same-length rewrite to `0x65` is mechanical.
    pub rewritable: u64,
    /// `fs`-prefixed instructions a linear sweep finds in uncovered code, by kind.
    /// They are code without unwind information, or, without a section table,
    /// possibly data that happens to decode.
    pub uncovered_fs: BTreeMap<FsAccess, u64>,
    /// Instructions inside covered ranges that do not decode.
    pub decode_failures: u64,
    pub tls: TlsModels,
}

impl ModuleScan {
    pub fn covered_fs(&self) -> u64 {
        self.fs_accesses.values().sum()
    }

    pub fn uncovered_total(&self) -> u64 {
        self.uncovered_fs.values().sum()
    }

    pub fn coverage(&self) -> f64 {
        if self.code_bytes == 0 {
            1.0
        } else {
            self.covered_bytes as f64 / self.code_bytes as f64
        }
    }

    /// True when T-001 could rewrite every `fs` access of the module mechanically:
    /// all found, all located, none outside the unwind tables, nothing undecodable.
    pub fn rewrite_is_clean(&self) -> bool {
        self.unwind_error.is_none()
            && self.uncovered_total() == 0
            && self.decode_failures == 0
            && self.rewritable == self.covered_fs()
    }
}

/// One scanned file of a directory scan.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Entry {
    /// Path relative to the scanned root, with `/` separators.
    pub path: String,
    pub result: Result<ModuleScan, String>,
}
