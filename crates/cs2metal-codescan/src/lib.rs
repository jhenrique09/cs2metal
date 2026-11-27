//! Read-only scan of guest x86-64 code for what the `fs` to `gs` rewrite (transform
//! T-001) and the TLS layer need to know.
//!
//! For each module the scan decodes every instruction inside the code ranges the
//! `.eh_frame` unwind tables describe, then discovers further code by following
//! direct calls and jumps from the module's own references to its code (see
//! [`Discovery`]). Each `fs`-prefixed instruction in proven code is classified, and
//! its `0x64` prefix byte located for a same-length rewrite. The code bytes no rule
//! proves are swept linearly only to count `fs` decodes there: those inside blocks
//! that code loads by address and that do not decode as code are data, the rest are
//! unexplained. Code means the executable sections when the object has a section
//! table, because some linkers map read-only data in the executable segment, and the
//! executable segments otherwise. The TLS models a module uses come from its dynamic
//! relocations and from its code.
//!
//! Nothing is modified. Reports derived from Valve's binaries stay local.

mod discover;
mod fs;
mod report;
mod scan;
mod walk;

use std::collections::BTreeMap;

use serde::Serialize;

pub use discover::{Discovery, Seed};
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
    /// A string instruction (`movs`, `cmps`, `outs`, ...) reading through `fs:rsi`,
    /// or an `fs` prefix on an instruction without a memory operand. Compilers do
    /// not emit these; in a linear sweep they mark data decoded as code.
    Unaddressed,
}

impl FsAccess {
    pub fn label(self) -> &'static str {
        match self {
            Self::ThreadPointer => "fs:0",
            Self::StackGuard => "fs:0x28",
            Self::PointerGuard => "fs:0x30",
            Self::ConstantOffset => "fs:constant",
            Self::RegisterRelative => "fs:register",
            Self::Unaddressed => "fs:unaddressed",
        }
    }

    pub const ALL: [Self; 6] = [
        Self::ThreadPointer,
        Self::StackGuard,
        Self::PointerGuard,
        Self::ConstantOffset,
        Self::RegisterRelative,
        Self::Unaddressed,
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
    /// The module is an executable (it has a program interpreter), the only kind
    /// of module that may use local-exec TLS.
    pub executable: bool,
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
        // Local-exec reaches a variable at a constant offset from the thread
        // pointer: either `fs:offset` directly, or, as gcc emits it, a load of the
        // thread pointer from `fs:0` followed by address arithmetic. Only an
        // executable with its own TLS block may do the second.
        let constant = code.get(&FsAccess::ConstantOffset).copied().unwrap_or(0) > 0;
        let thread_pointer = code.get(&FsAccess::ThreadPointer).copied().unwrap_or(0) > 0;
        if constant || (self.executable && self.tls_block_bytes > 0 && thread_pointer) {
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
    /// Code reached beyond the unwind tables by following direct calls and jumps,
    /// and the `fs` accesses found in it.
    pub discovery: Discovery,
    /// `fs`-prefixed instructions a linear sweep finds in the code bytes that
    /// neither the unwind tables nor discovery reach, by kind. They are code no
    /// rule proves, or data that happens to decode: unexplained either way.
    pub uncovered_fs: BTreeMap<FsAccess, u64>,
    /// `fs`-prefixed decodes in unexplained bytes at or after an address that
    /// accepted code loads with `lea` and that does not decode as code: data the
    /// code refers to, such as tables kept in `.text`.
    pub data_fs: BTreeMap<FsAccess, u64>,
    /// Instructions inside covered ranges that do not decode.
    pub decode_failures: u64,
    pub tls: TlsModels,
}

impl ModuleScan {
    pub fn covered_fs(&self) -> u64 {
        self.fs_accesses.values().sum()
    }

    /// The TLS models in use, from relocations and from every `fs` access in
    /// proven code, unwind-described or discovered.
    pub fn tls_summary(&self) -> Vec<&'static str> {
        let mut code = self.fs_accesses.clone();
        for (kind, n) in &self.discovery.fs_accesses {
            *code.entry(*kind).or_default() += n;
        }
        self.tls.summary(&code)
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

    /// Unexplained `fs` decodes that could be real accesses: every kind except
    /// string instructions, which compilers do not emit.
    pub fn unexplained_addressed(&self) -> u64 {
        self.uncovered_fs
            .iter()
            .filter(|(k, _)| **k != FsAccess::Unaddressed)
            .map(|(_, n)| n)
            .sum()
    }

    /// True when T-001 could rewrite every `fs` access of the module mechanically:
    /// all found in code the unwind tables or discovery prove, all located, nothing
    /// undecodable, and no unexplained decode that could be a real access.
    pub fn rewrite_is_clean(&self) -> bool {
        self.unwind_error.is_none()
            && self.unexplained_addressed() == 0
            && self.decode_failures == 0
            && self.rewritable == self.covered_fs()
            && self.discovery.rewritable == self.discovery.fs_total()
    }
}

/// One scanned file of a directory scan.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Entry {
    /// Path relative to the scanned root, with `/` separators.
    pub path: String,
    pub result: Result<ModuleScan, String>,
}
