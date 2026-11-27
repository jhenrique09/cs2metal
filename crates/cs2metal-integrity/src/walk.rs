use std::fs;
use std::path::Path;
use std::time::UNIX_EPOCH;

use crate::{Error, LinkRecord};

/// Top-level directories that are not part of the game: `steamapps/` holds the
/// download tool's own bookkeeping (app manifests, download state), which it
/// rewrites on every update and verification.
pub const EXCLUDED_TOP_LEVEL: &[&str] = &["steamapps"];

#[derive(Debug, Clone)]
pub(crate) struct FileEntry {
    pub path: String,
    pub size: u64,
    pub modified_ns: u128,
}

#[derive(Debug, Default)]
pub(crate) struct Tree {
    pub files: Vec<FileEntry>,
    pub links: Vec<LinkRecord>,
}

pub(crate) fn walk(root: &Path) -> Result<Tree, Error> {
    let meta = fs::metadata(root).map_err(|e| Error::Io(root.to_owned(), e))?;
    if !meta.is_dir() {
        return Err(Error::NotADirectory(root.to_owned()));
    }
    let mut tree = Tree::default();
    visit(root, root, &mut tree)?;
    tree.files.sort_by(|a, b| a.path.cmp(&b.path));
    tree.links.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(tree)
}

fn relative(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .components()
        .map(|c| c.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}

fn visit(root: &Path, dir: &Path, tree: &mut Tree) -> Result<(), Error> {
    let entries = fs::read_dir(dir).map_err(|e| Error::Io(dir.to_owned(), e))?;
    for entry in entries {
        let entry = entry.map_err(|e| Error::Io(dir.to_owned(), e))?;
        let path = entry.path();
        let rel = relative(root, &path);
        if dir == root && EXCLUDED_TOP_LEVEL.contains(&rel.as_str()) {
            continue;
        }
        // symlink_metadata never follows a link.
        let meta = fs::symlink_metadata(&path).map_err(|e| Error::Io(path.clone(), e))?;
        let kind = meta.file_type();
        if kind.is_symlink() {
            let target = fs::read_link(&path).map_err(|e| Error::Io(path.clone(), e))?;
            tree.links.push(LinkRecord {
                path: rel,
                target: target.to_string_lossy().into_owned(),
            });
        } else if kind.is_dir() {
            visit(root, &path, tree)?;
        } else if kind.is_file() {
            let modified_ns = meta
                .modified()
                .ok()
                .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                .map_or(0, |d| d.as_nanos());
            tree.files.push(FileEntry {
                path: rel,
                size: meta.len(),
                modified_ns,
            });
        }
    }
    Ok(())
}
