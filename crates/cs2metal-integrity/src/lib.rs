//! The integrity guard: a SHA-256 baseline of every file in a game installation,
//! recorded once after a clean `validate` and verified before cs2metal touches the
//! installation again.
//!
//! Everything here is read-only. Files are opened for reading only, directories
//! are listed, symbolic links are read with `readlink` and never followed. The
//! baseline stores paths relative to the installation root, so it never contains
//! a machine-specific path.

mod hash;
mod walk;

use std::collections::BTreeMap;
use std::fmt;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicU64;

use serde::{Deserialize, Serialize};

pub use walk::EXCLUDED_TOP_LEVEL;

/// The on-disk format version written into every baseline.
pub const FORMAT: u32 = 1;

/// One regular file of the installation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileRecord {
    /// Path relative to the root, with `/` separators.
    pub path: String,
    pub size: u64,
    /// Modification time in nanoseconds since the Unix epoch, used only by the
    /// quick verification mode to decide which files to hash again.
    pub modified_ns: u128,
    /// Lowercase hexadecimal SHA-256 of the contents.
    pub sha256: String,
}

/// One symbolic link of the installation, recorded without following it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LinkRecord {
    pub path: String,
    pub target: String,
}

/// A recorded baseline.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Baseline {
    pub format: u32,
    /// Seconds since the Unix epoch when the baseline was recorded.
    pub recorded_at: u64,
    /// Top-level directories left out of the baseline, and why they are.
    pub excluded: Vec<String>,
    pub files: Vec<FileRecord>,
    pub links: Vec<LinkRecord>,
}

impl Baseline {
    pub fn total_bytes(&self) -> u64 {
        self.files.iter().map(|f| f.size).sum()
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).unwrap_or_default()
    }

    pub fn from_json(text: &str) -> Result<Self, Error> {
        let baseline: Self =
            serde_json::from_str(text).map_err(|e| Error::Format(e.to_string()))?;
        if baseline.format != FORMAT {
            return Err(Error::Format(format!(
                "baseline format {} is not the supported format {FORMAT}",
                baseline.format
            )));
        }
        Ok(baseline)
    }
}

/// How thoroughly `verify` compares.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// Hash every file again.
    Full,
    /// Hash again only the files whose size or modification time changed.
    /// Fast, and blind to a same-size change that also restores the timestamp.
    Quick,
}

/// One difference between a baseline and the installation.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum Difference {
    Missing(String),
    Added(String),
    Modified(String),
    LinkChanged {
        path: String,
        expected: String,
        found: String,
    },
    LinkMissing(String),
    LinkAdded(String),
}

impl fmt::Display for Difference {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Missing(p) => write!(f, "missing: {p}"),
            Self::Added(p) => write!(f, "added: {p}"),
            Self::Modified(p) => write!(f, "modified: {p}"),
            Self::LinkChanged {
                path,
                expected,
                found,
            } => write!(f, "link changed: {path} -> {found} (expected {expected})"),
            Self::LinkMissing(p) => write!(f, "link missing: {p}"),
            Self::LinkAdded(p) => write!(f, "link added: {p}"),
        }
    }
}

/// The outcome of a verification.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Report {
    pub differences: Vec<Difference>,
    pub files_checked: usize,
    pub files_hashed: usize,
    pub bytes_hashed: u64,
}

impl Report {
    pub fn is_clean(&self) -> bool {
        self.differences.is_empty()
    }
}

#[derive(Debug)]
pub enum Error {
    NotADirectory(PathBuf),
    Io(PathBuf, io::Error),
    Format(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotADirectory(p) => write!(f, "{} is not a directory", p.display()),
            Self::Io(p, e) => write!(f, "{}: {e}", p.display()),
            Self::Format(m) => write!(f, "invalid baseline: {m}"),
        }
    }
}

impl std::error::Error for Error {}

/// Records a baseline of `root`. `progress` receives the number of bytes hashed so
/// far, for callers that report progress from another thread.
pub fn record(root: &Path, progress: &AtomicU64) -> Result<Baseline, Error> {
    let tree = walk::walk(root)?;
    let files = hash::hash_all(root, &tree.files, progress)?;
    let recorded_at = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    Ok(Baseline {
        format: FORMAT,
        recorded_at,
        excluded: EXCLUDED_TOP_LEVEL.iter().map(|s| (*s).to_owned()).collect(),
        files,
        links: tree.links,
    })
}

/// Compares `root` with `baseline`.
pub fn verify(
    root: &Path,
    baseline: &Baseline,
    mode: Mode,
    progress: &AtomicU64,
) -> Result<Report, Error> {
    let tree = walk::walk(root)?;
    let expected: BTreeMap<&str, &FileRecord> = baseline
        .files
        .iter()
        .map(|f| (f.path.as_str(), f))
        .collect();
    let found: BTreeMap<&str, &walk::FileEntry> =
        tree.files.iter().map(|f| (f.path.as_str(), f)).collect();

    let mut report = Report {
        files_checked: found.len(),
        ..Report::default()
    };
    let mut to_hash = Vec::new();
    for (path, record) in &expected {
        match found.get(path) {
            None => report
                .differences
                .push(Difference::Missing((*path).to_owned())),
            Some(entry) if entry.size != record.size => {
                report
                    .differences
                    .push(Difference::Modified((*path).to_owned()));
            }
            Some(entry) => {
                if mode == Mode::Full || entry.modified_ns != record.modified_ns {
                    to_hash.push((*entry).clone());
                }
            }
        }
    }
    for path in found.keys() {
        if !expected.contains_key(path) {
            report
                .differences
                .push(Difference::Added((*path).to_owned()));
        }
    }

    let hashed = hash::hash_all(root, &to_hash, progress)?;
    report.files_hashed = hashed.len();
    report.bytes_hashed = hashed.iter().map(|f| f.size).sum();
    for file in hashed {
        if expected.get(file.path.as_str()).map(|r| r.sha256.as_str()) != Some(file.sha256.as_str())
        {
            report.differences.push(Difference::Modified(file.path));
        }
    }

    let expected_links: BTreeMap<&str, &str> = baseline
        .links
        .iter()
        .map(|l| (l.path.as_str(), l.target.as_str()))
        .collect();
    let found_links: BTreeMap<&str, &str> = tree
        .links
        .iter()
        .map(|l| (l.path.as_str(), l.target.as_str()))
        .collect();
    for (path, target) in &expected_links {
        match found_links.get(path) {
            None => report
                .differences
                .push(Difference::LinkMissing((*path).to_owned())),
            Some(found) if found != target => report.differences.push(Difference::LinkChanged {
                path: (*path).to_owned(),
                expected: (*target).to_owned(),
                found: (*found).to_owned(),
            }),
            Some(_) => {}
        }
    }
    for path in found_links.keys() {
        if !expected_links.contains_key(path) {
            report
                .differences
                .push(Difference::LinkAdded((*path).to_owned()));
        }
    }
    report.differences.sort();
    Ok(report)
}
