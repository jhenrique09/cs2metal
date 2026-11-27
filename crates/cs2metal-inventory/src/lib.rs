//! Scans a CS2 Linux installation read-only and answers one question: which
//! libraries and symbols must the host provide, because no bundled module does?
//!
//! Every ELF object under the root is parsed. Each import is resolved the way
//! glibc's loader would find it among the bundled modules: by the library its
//! symbol version names, or else through the importing module's `DT_NEEDED`
//! closure. What stays unresolved is grouped by host library and library family,
//! which is the work list for the glibc ABI layer and the substitution table.

mod family;
mod report;
mod scan;

use std::collections::{BTreeMap, BTreeSet};

use serde::Serialize;

pub use family::Family;
pub use report::markdown;
pub use scan::{Error, needed_closure, scan};

/// The result of scanning one installation.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Inventory {
    /// Every ELF object found, sorted by relative path.
    pub modules: Vec<Module>,
    /// Files that start with the ELF magic but could not be parsed.
    pub skipped: Vec<Skipped>,
    /// Libraries named by `DT_NEEDED` or a version requirement that no bundled
    /// module provides.
    pub host_libraries: Vec<HostLibrary>,
    /// Unversioned imports that neither a bundled module nor a versioned host
    /// library accounts for.
    pub unattributed: BTreeMap<String, Demand>,
    /// Versioned imports of a host library that a bundled module also exports, so
    /// the bundled definition wins (symbol to exporting module).
    pub interposed: BTreeMap<String, String>,
    /// Relocation counts by type name, over every module.
    pub relocations: BTreeMap<String, u64>,
    /// Program interpreters requested by executables.
    pub interpreters: BTreeSet<String>,
}

/// One bundled ELF object.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Module {
    /// Path relative to the scanned root, with `/` separators.
    pub path: String,
    pub soname: Option<String>,
    pub object_type: cs2metal_elf::ObjectType,
    pub machine: String,
    pub pie: bool,
    pub needed: Vec<Needed>,
    pub imports: usize,
    pub exports: usize,
    /// Exports that are GNU indirect functions, whose resolvers the loader calls.
    pub ifunc_exports: usize,
    /// The module has a `PT_TLS` segment or uses the initial-exec TLS model.
    pub uses_tls: bool,
    pub static_tls: bool,
    pub copy_relocations: Vec<String>,
    pub text_relocations: bool,
}

/// One `DT_NEEDED` entry and who provides it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Needed {
    pub name: String,
    /// The relative path of the bundled module that provides it, if any.
    pub bundled: Option<String>,
}

/// A library the host must provide.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct HostLibrary {
    pub name: String,
    pub family: Family,
    /// Relative paths of the modules that need it.
    pub required_by: BTreeSet<String>,
    /// Symbols attributed to this library through their version requirement.
    pub symbols: BTreeMap<String, Demand>,
}

/// How a symbol is demanded across modules.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Demand {
    /// Version names requested (empty for unversioned references).
    pub versions: BTreeSet<String>,
    /// True when every reference is weak, so the symbol may stay unbound.
    pub weak_only: bool,
    pub referenced_by: BTreeSet<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Skipped {
    pub path: String,
    pub reason: String,
}

impl Inventory {
    pub fn host_library(&self, name: &str) -> Option<&HostLibrary> {
        self.host_libraries.iter().find(|l| l.name == name)
    }

    pub fn module(&self, path: &str) -> Option<&Module> {
        self.modules.iter().find(|m| m.path == path)
    }

    /// Host symbols that must be implemented: every attributed symbol plus the
    /// unattributed ones, excluding references that are weak everywhere.
    pub fn required_symbol_count(&self) -> usize {
        let attributed = self
            .host_libraries
            .iter()
            .flat_map(|l| l.symbols.values())
            .filter(|d| !d.weak_only)
            .count();
        attributed + self.unattributed.values().filter(|d| !d.weak_only).count()
    }
}
