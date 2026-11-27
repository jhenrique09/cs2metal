use std::fs;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use crate::{Entry, scan_module};

fn is_elf(path: &Path) -> io::Result<bool> {
    let mut magic = [0u8; 4];
    match fs::File::open(path)?.read_exact(&mut magic) {
        Ok(()) => Ok(&magic == b"\x7fELF"),
        Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => Ok(false),
        Err(e) => Err(e),
    }
}

fn collect(root: &Path, dir: &Path, out: &mut Vec<(String, PathBuf)>) -> io::Result<()> {
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        let kind = entry.file_type()?;
        if kind.is_dir() {
            collect(root, &path, out)?;
        } else if kind.is_file() && is_elf(&path)? {
            let relative = path.strip_prefix(root).unwrap_or(&path);
            let name = relative
                .components()
                .map(|c| c.as_os_str().to_string_lossy())
                .collect::<Vec<_>>()
                .join("/");
            out.push((name, path));
        }
    }
    Ok(())
}

fn scan_file(path: &Path) -> Result<crate::ModuleScan, String> {
    let bytes = fs::read(path).map_err(|e| e.to_string())?;
    let elf = cs2metal_elf::parse(&bytes).map_err(|e| e.to_string())?;
    Ok(scan_module(&bytes, &elf))
}

/// Scans one ELF file, or every ELF file under a directory, on all cores.
/// Symbolic links are not followed. Entries are sorted by relative path; a single
/// file is reported under its own name.
pub fn scan_path(path: &Path) -> io::Result<Vec<Entry>> {
    let meta = fs::metadata(path)?;
    let mut files = Vec::new();
    if meta.is_dir() {
        collect(path, path, &mut files)?;
    } else {
        let name = path
            .file_name()
            .map_or_else(String::new, |n| n.to_string_lossy().into_owned());
        files.push((name, path.to_owned()));
    }
    files.sort();

    let next = AtomicUsize::new(0);
    let results: Mutex<Vec<Option<Entry>>> = Mutex::new((0..files.len()).map(|_| None).collect());
    let workers = std::thread::available_parallelism()
        .map_or(4, |n| n.get())
        .min(files.len().max(1));
    std::thread::scope(|scope| {
        for _ in 0..workers {
            scope.spawn(|| {
                loop {
                    let index = next.fetch_add(1, Ordering::Relaxed);
                    let Some((name, file)) = files.get(index) else {
                        break;
                    };
                    let entry = Entry {
                        path: name.clone(),
                        result: scan_file(file),
                    };
                    if let Ok(mut slots) = results.lock() {
                        slots[index] = Some(entry);
                    }
                }
            });
        }
    });
    Ok(results
        .into_inner()
        .unwrap_or_default()
        .into_iter()
        .flatten()
        .collect())
}
