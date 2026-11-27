//! Guest paths as the host should use them: the shims' virtual paths, and the
//! installation overlay.
//!
//! The game writes into its own installation: `steam_appid.txt`, configuration,
//! logs, caches. cs2metal never lets a write reach the installation. Every guest
//! path inside it is looked up first in an overlay directory outside it; a write
//! goes to the overlay, copying the original up first when its contents are kept;
//! deleting an original file is refused with `EROFS`. Reads of anything the game
//! has not written fall through to the installation, which stays byte-identical to
//! its baseline.

use std::borrow::Cow;
use std::ffi::{CStr, CString};
use std::os::unix::ffi::OsStrExt;
use std::path::{Component, Path, PathBuf};
use std::sync::OnceLock;

/// The installation and the overlay that receives its writes.
#[derive(Debug, Clone)]
pub struct Overlay {
    pub installation: PathBuf,
    pub upper: PathBuf,
}

static OVERLAY: OnceLock<Overlay> = OnceLock::new();

pub(crate) fn set_overlay(overlay: Overlay) {
    let _ = OVERLAY.set(overlay);
}

/// What the guest does with a path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Use {
    Read,
    /// A write that keeps existing contents, such as `O_RDWR` without `O_TRUNC`.
    Modify,
    /// A write that replaces or creates, so nothing needs copying up.
    Replace,
    /// Removing the entry.
    Remove,
}

/// The `Use` of an `open` with Linux flags.
pub(crate) fn open_use(flags: u64) -> Use {
    const WRITE: u64 = 0o1 | 0o2 | 0o100 | 0o1000 | 0o2000; // WRONLY RDWR CREAT TRUNC APPEND
    if flags & WRITE == 0 {
        Use::Read
    } else if flags & 0o1000 != 0 {
        Use::Replace
    } else {
        Use::Modify
    }
}

/// An absolute, lexically normalized form of `path`, relative ones resolved
/// against `base`.
fn absolute(path: &Path, base: &Path) -> PathBuf {
    let joined = if path.is_absolute() {
        path.to_owned()
    } else {
        base.join(path)
    };
    let mut out = PathBuf::from("/");
    for c in joined.components() {
        match c {
            Component::ParentDir => {
                out.pop();
            }
            Component::Normal(n) => out.push(n),
            _ => {}
        }
    }
    out
}

/// The directory a `dirfd` names, or the working directory for `AT_FDCWD`.
fn directory_of(dirfd: libc::c_int) -> Option<PathBuf> {
    if dirfd == libc::AT_FDCWD {
        return std::env::current_dir().ok();
    }
    let mut buf = vec![0u8; libc::PATH_MAX as usize];
    // SAFETY: F_GETPATH writes at most PATH_MAX bytes into the buffer.
    let status = unsafe { libc::fcntl(dirfd, libc::F_GETPATH, buf.as_mut_ptr()) };
    if status < 0 {
        return None;
    }
    let end = buf.iter().position(|&b| b == 0)?;
    Some(PathBuf::from(std::ffi::OsStr::from_bytes(&buf[..end])))
}

fn c_path(p: &Path) -> Option<CString> {
    CString::new(p.as_os_str().as_bytes()).ok()
}

/// Where the host should look for a guest path, or a Linux errno.
pub(crate) fn resolve<'a>(
    path: &'a CStr,
    dirfd: libc::c_int,
    how: Use,
) -> Result<Cow<'a, CStr>, i64> {
    if let Some(shim) = crate::steam::redirect(path) {
        return Ok(Cow::Owned(shim));
    }
    let Some(overlay) = OVERLAY.get() else {
        return Ok(Cow::Borrowed(path));
    };
    let raw = Path::new(std::ffi::OsStr::from_bytes(path.to_bytes()));
    let base = if raw.is_absolute() {
        PathBuf::from("/")
    } else {
        match directory_of(dirfd) {
            Some(d) => d,
            None => return Ok(Cow::Borrowed(path)),
        }
    };
    let full = absolute(raw, &base);
    let Ok(relative) = full.strip_prefix(&overlay.installation) else {
        // A path inside the overlay, or anywhere else, is used as it is.
        return Ok(Cow::Borrowed(path));
    };
    let upper = overlay.upper.join(relative);
    let in_upper = match std::fs::symlink_metadata(&upper) {
        // A directory the installation has too is read from the installation:
        // the overlay holds only the parents of what the game wrote, and listing
        // it instead would hide the installation's contents. (Listings do not yet
        // merge the two.)
        Ok(meta) if meta.is_dir() && how == Use::Read => full.symlink_metadata().is_err(),
        Ok(_) => true,
        Err(_) => false,
    };
    let chosen = match (how, in_upper) {
        (_, true) => upper,
        (Use::Read, false) => full,
        (Use::Remove, false) => {
            return Err(if std::fs::symlink_metadata(&full).is_ok() {
                -30 // EROFS: the installation is read-only to the guest
            } else {
                crate::translate::ENOENT
            });
        }
        (Use::Modify | Use::Replace, false) => {
            if let Some(parent) = upper.parent() {
                if std::fs::create_dir_all(parent).is_err() {
                    return Err(-13); // EACCES
                }
            }
            if how == Use::Modify && full.is_file() && std::fs::copy(&full, &upper).is_err() {
                return Err(-13);
            }
            upper
        }
    };
    c_path(&chosen)
        .map(Cow::Owned)
        .ok_or(crate::translate::EFAULT)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_go_to_the_overlay_and_reads_fall_through() {
        let root = std::env::temp_dir().join(format!("cs2metal-overlay-{}", std::process::id()));
        let installation = root.join("install");
        let upper = root.join("upper");
        std::fs::create_dir_all(installation.join("game/cfg")).expect("mkdir");
        std::fs::write(installation.join("game/cfg/a.cfg"), b"original").expect("write");
        let _ = OVERLAY.set(Overlay {
            installation: installation.clone(),
            upper: upper.clone(),
        });
        let path = |p: &Path| CString::new(p.as_os_str().as_bytes()).expect("path");
        let original = path(&installation.join("game/cfg/a.cfg"));

        let read = resolve(&original, libc::AT_FDCWD, Use::Read).expect("read");
        assert_eq!(read.to_bytes(), original.to_bytes(), "reads fall through");

        let modified = resolve(&original, libc::AT_FDCWD, Use::Modify).expect("modify");
        assert_eq!(
            modified.to_bytes(),
            path(&upper.join("game/cfg/a.cfg")).to_bytes()
        );
        assert_eq!(
            std::fs::read(upper.join("game/cfg/a.cfg")).expect("copied"),
            b"original"
        );

        let again = resolve(&original, libc::AT_FDCWD, Use::Read).expect("read");
        assert_eq!(
            again.to_bytes(),
            modified.to_bytes(),
            "the copy is seen from now on"
        );

        let fresh = path(&installation.join("game/bin/steam_appid.txt"));
        let created = resolve(&fresh, libc::AT_FDCWD, Use::Replace).expect("create");
        assert!(created.to_bytes().starts_with(path(&upper).to_bytes()));

        // The overlay now has game/ and game/cfg/ for the copy above; reading those
        // directories still means the installation's.
        let dir = path(&installation.join("game/cfg"));
        let listed = resolve(&dir, libc::AT_FDCWD, Use::Read).expect("dir");
        assert_eq!(
            listed.to_bytes(),
            dir.to_bytes(),
            "a shared directory reads from below"
        );

        let other = path(&installation.join("game/cfg/b.cfg"));
        std::fs::write(installation.join("game/cfg/b.cfg"), b"x").expect("write");
        assert_eq!(
            resolve(&other, libc::AT_FDCWD, Use::Remove).err(),
            Some(-30)
        );
        assert_eq!(
            std::fs::read(installation.join("game/cfg/a.cfg")).expect("read"),
            b"original"
        );
        let _ = std::fs::remove_dir_all(root);
    }
}
