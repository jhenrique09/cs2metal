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

/// The installation's path with its symbolic links resolved, as the host names a
/// descriptor's file (`F_GETPATH`) and the working directory.
static CANONICAL: OnceLock<PathBuf> = OnceLock::new();

pub(crate) fn set_overlay(overlay: Overlay) {
    if let Ok(canonical) = std::fs::canonicalize(&overlay.installation) {
        let _ = CANONICAL.set(canonical);
    }
    let _ = OVERLAY.set(overlay);
}

/// `path` relative to the installation, under either of its names.
fn inside_installation<'p>(path: &'p Path, overlay: &Overlay) -> Option<&'p Path> {
    path.strip_prefix(&overlay.installation)
        .ok()
        .or_else(|| path.strip_prefix(CANONICAL.get()?).ok())
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

/// Whether the open descriptor `fd` is a file of the installation itself (not of
/// the overlay), which no change of metadata may reach.
pub(crate) fn is_installation_file(fd: libc::c_int) -> bool {
    let Some(overlay) = OVERLAY.get() else {
        return false;
    };
    directory_of(fd).is_some_and(|p| inside_installation(&p, overlay).is_some())
}

fn c_path(p: &Path) -> Option<CString> {
    CString::new(p.as_os_str().as_bytes()).ok()
}

/// Whether `CS2METAL_NO_INTRO` asks for the game's opening movie to be left out.
fn no_intro() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("CS2METAL_NO_INTRO").is_some_and(|v| v != "0"))
}

/// Whether `path` names one of the game's opening movies: the `intro` files of
/// its Panorama videos, in every size and variant.
fn is_intro_movie(path: &[u8]) -> bool {
    const DIRECTORY: &[u8] = b"/panorama/videos/";
    let Some(at) = path.windows(DIRECTORY.len()).rposition(|w| w == DIRECTORY) else {
        return false;
    };
    let name = &path[at + DIRECTORY.len()..];
    name.starts_with(b"intro") && name.ends_with(b".webm") && !name.contains(&b'/')
}

/// Where the host should look for a guest path, or a Linux errno.
pub(crate) fn resolve<'a>(
    path: &'a CStr,
    dirfd: libc::c_int,
    how: Use,
) -> Result<Cow<'a, CStr>, i64> {
    // The game does not skip its opening movie by any option of its own in
    // this build. Asked not to find the movie, it goes on to its menu; the
    // file itself is not touched.
    if no_intro() && is_intro_movie(path.to_bytes()) {
        return Err(crate::translate::ENOENT);
    }
    if let Some(shim) = crate::steam::redirect(path)
        .or_else(|| crate::sdl::redirect(path))
        .or_else(|| crate::vulkan::redirect(path))
    {
        return Ok(Cow::Owned(shim));
    }
    if how == Use::Read {
        if let Some(kernel) = crate::procfs::synthesize(path.to_bytes()) {
            return Ok(Cow::Owned(kernel));
        }
        if let Some(pid) = crate::steam::pid_file(path) {
            return Ok(Cow::Owned(pid));
        }
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
    let Some(relative) = inside_installation(&full, overlay) else {
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
    fn the_opening_movies_are_the_intro_files_of_the_panorama_videos() {
        assert!(is_intro_movie(b"/game/csgo/panorama/videos/intro.webm"));
        assert!(is_intro_movie(b"/game/csgo/panorama/videos/intro720p.webm"));
        assert!(is_intro_movie(
            b"panorama/videos/../../panorama/videos/intro-perfectworld.webm"
        ));
        assert!(!is_intro_movie(
            b"/game/csgo/panorama/videos/trailer_0.webm"
        ));
        assert!(!is_intro_movie(
            b"/game/csgo/panorama/videos/intro/other.webm"
        ));
        assert!(!is_intro_movie(b"/game/csgo/panorama/videos/intro.txt"));
        assert!(!is_intro_movie(b"/game/csgo/videos/intro.webm"));
        assert!(!is_intro_movie(b""));
    }

    #[test]
    fn writes_go_to_the_overlay_and_reads_fall_through() {
        let root = std::env::temp_dir().join(format!("cs2metal-overlay-{}", std::process::id()));
        let installation = root.join("install");
        let upper = root.join("upper");
        std::fs::create_dir_all(installation.join("game/cfg")).expect("mkdir");
        std::fs::write(installation.join("game/cfg/a.cfg"), b"original").expect("write");
        set_overlay(Overlay {
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

        // A descriptor of an installation file is recognized, so that fchmod can
        // refuse it; the overlay's copy is not.
        let below = std::fs::File::open(installation.join("game/cfg/b.cfg")).expect("open");
        let above = std::fs::File::open(upper.join("game/cfg/a.cfg")).expect("open");
        use std::os::fd::AsRawFd;
        assert!(is_installation_file(below.as_raw_fd()));
        assert!(!is_installation_file(above.as_raw_fd()));

        // A path relative to a directory descriptor, which the host names by its
        // canonical path, is still inside the installation.
        let dir = std::fs::File::open(installation.join("game")).expect("open");
        let relative = CString::new("cfg/b.cfg").expect("path");
        let written = resolve(&relative, dir.as_raw_fd(), Use::Replace).expect("write");
        assert!(
            written.to_bytes().ends_with(b"upper/game/cfg/b.cfg"),
            "{written:?} goes to the overlay"
        );
        let _ = std::fs::remove_dir_all(root);
    }
}
