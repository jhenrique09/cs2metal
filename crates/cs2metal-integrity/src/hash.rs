use std::fmt::Write as _;
use std::fs::File;
use std::io::Read;
use std::path::Path;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

use sha2::{Digest, Sha256};

use crate::walk::FileEntry;
use crate::{Error, FileRecord};

const BUFFER: usize = 1 << 20;

fn hex(bytes: &[u8]) -> String {
    bytes.iter().fold(String::with_capacity(64), |mut s, b| {
        let _ = write!(s, "{b:02x}");
        s
    })
}

fn hash_file(path: &Path, progress: &AtomicU64) -> Result<String, Error> {
    // File::open opens read-only.
    let mut file = File::open(path).map_err(|e| Error::Io(path.to_owned(), e))?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; BUFFER];
    loop {
        let n = file
            .read(&mut buffer)
            .map_err(|e| Error::Io(path.to_owned(), e))?;
        if n == 0 {
            break;
        }
        hasher.update(&buffer[..n]);
        progress.fetch_add(n as u64, Ordering::Relaxed);
    }
    Ok(hex(&hasher.finalize()))
}

/// Hashes `entries` on every available core. The result is in the order of
/// `entries`, whatever order the workers finish in.
pub(crate) fn hash_all(
    root: &Path,
    entries: &[FileEntry],
    progress: &AtomicU64,
) -> Result<Vec<FileRecord>, Error> {
    let workers = std::thread::available_parallelism()
        .map_or(4, |n| n.get())
        .min(entries.len().max(1));
    let next = AtomicUsize::new(0);
    let results: Mutex<Vec<Option<Result<String, Error>>>> =
        Mutex::new((0..entries.len()).map(|_| None).collect());

    std::thread::scope(|scope| {
        for _ in 0..workers {
            scope.spawn(|| {
                loop {
                    let index = next.fetch_add(1, Ordering::Relaxed);
                    let Some(entry) = entries.get(index) else {
                        break;
                    };
                    let digest = hash_file(&root.join(&entry.path), progress);
                    let failed = digest.is_err();
                    if let Ok(mut slots) = results.lock() {
                        slots[index] = Some(digest);
                    }
                    if failed {
                        // Stop handing out work; the first error is reported below.
                        next.store(entries.len(), Ordering::Relaxed);
                    }
                }
            });
        }
    });

    let slots = results.into_inner().unwrap_or_default();
    let mut out = Vec::with_capacity(entries.len());
    for (entry, slot) in entries.iter().zip(slots) {
        match slot {
            Some(Ok(sha256)) => out.push(FileRecord {
                path: entry.path.clone(),
                size: entry.size,
                modified_ns: entry.modified_ns,
                sha256,
            }),
            Some(Err(e)) => return Err(e),
            None => {}
        }
    }
    Ok(out)
}
