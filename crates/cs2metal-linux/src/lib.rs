//! Starts a Linux program under its own, rebuilt glibc, and answers its system
//! calls on macOS, by the [libc route decision](../../../docs/LIBC_ROUTE_DECISION.md).
//!
//! The host does what the Linux kernel does when it runs `ld.so` as a program:
//! it maps ld.so (with transform T-001), builds the initial stack and auxiliary
//! vector, and enters it with GS at a bootstrap control block whose system call
//! slot points at the bridge. ld.so then loads the program and its libraries
//! itself, through system calls this crate answers; each executable file mapping
//! gets T-001 as it is made.

mod brk;
mod epoll;
mod fds;
mod ipc;
mod mmap;
mod more;
mod net;
mod paths;
mod procfs;
mod sdl;
mod steam;
mod syscall;
mod threads;
mod translate;
mod vulkan;

use std::convert::Infallible;
use std::ffi::CString;
use std::fmt;
use std::path::{Path, PathBuf};

pub use paths::Overlay;

use cs2metal_loader::{FsRewrite, HostBlock, Image, attach, enter_process, set_syscall_handler};

/// A Linux program to start.
#[derive(Debug, Clone)]
pub struct Launch {
    /// The rebuilt glibc's `ld-linux-x86-64.so.2`.
    pub ld_so: PathBuf,
    /// Directories ld.so searches first, as `--library-path`: the rebuilt glibc's,
    /// then the program's own.
    pub library_path: Vec<PathBuf>,
    pub program: PathBuf,
    /// Arguments after the program name.
    pub args: Vec<String>,
    /// The guest's environment, `NAME=value`.
    pub env: Vec<String>,
    /// Where the guest's writes into an installation go instead.
    pub overlay: Option<Overlay>,
}

/// Why a program could not be started.
#[derive(Debug)]
pub enum Error {
    Loader(cs2metal_loader::Error),
    /// The program or a path holds a NUL byte, or cannot be made absolute.
    BadPath(PathBuf),
    /// Guest code runs only in an x86-64 process.
    ForeignHost,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Loader(e) => write!(f, "{e}"),
            Self::BadPath(p) => write!(f, "unusable path {}", p.display()),
            Self::ForeignHost => write!(f, "guest code runs only in an x86-64 process"),
        }
    }
}

impl std::error::Error for Error {}

impl From<cs2metal_loader::Error> for Error {
    fn from(e: cs2metal_loader::Error) -> Self {
        Self::Loader(e)
    }
}

const STACK_SIZE: usize = 8 << 20;

/// The auxiliary vector entry types the start uses.
mod at {
    pub const NULL: u64 = 0;
    pub const PHDR: u64 = 3;
    pub const PHENT: u64 = 4;
    pub const PHNUM: u64 = 5;
    pub const PAGESZ: u64 = 6;
    pub const BASE: u64 = 7;
    pub const FLAGS: u64 = 8;
    pub const ENTRY: u64 = 9;
    pub const UID: u64 = 11;
    pub const EUID: u64 = 12;
    pub const GID: u64 = 13;
    pub const EGID: u64 = 14;
    pub const PLATFORM: u64 = 15;
    pub const HWCAP: u64 = 16;
    pub const CLKTCK: u64 = 17;
    pub const SECURE: u64 = 23;
    pub const RANDOM: u64 = 25;
    pub const HWCAP2: u64 = 26;
    pub const EXECFN: u64 = 31;
}

fn c_string(s: &str, path: &Path) -> Result<CString, Error> {
    CString::new(s).map_err(|_| Error::BadPath(path.to_owned()))
}

/// Builds a Linux initial stack in `stack` (its top at `top`): strings, then the
/// auxiliary vector, environment, arguments, and argc, 16-byte aligned. Returns
/// the stack pointer to enter with.
fn initial_stack(top: u64, argv: &[CString], env: &[CString], aux: &[(u64, u64)]) -> u64 {
    let mut sp = top;
    let mut push_bytes = |bytes: &[u8]| -> u64 {
        sp -= bytes.len() as u64;
        // SAFETY: sp stays inside the stack mapping, which is far larger than the
        // strings, vectors, and random bytes placed at its top.
        unsafe { std::ptr::copy_nonoverlapping(bytes.as_ptr(), sp as *mut u8, bytes.len()) };
        sp
    };
    let mut random = [0u8; 16];
    // SAFETY: arc4random_buf fills the buffer.
    unsafe { libc::arc4random_buf(random.as_mut_ptr().cast(), random.len()) };
    let random = push_bytes(&random);
    let platform = push_bytes(b"x86_64\0");
    let env_ptrs: Vec<u64> = env
        .iter()
        .map(|s| push_bytes(s.as_bytes_with_nul()))
        .collect();
    let arg_ptrs: Vec<u64> = argv
        .iter()
        .map(|s| push_bytes(s.as_bytes_with_nul()))
        .collect();
    let execfn = arg_ptrs.first().copied().unwrap_or(0);

    let mut words: Vec<u64> = vec![argv.len() as u64];
    words.extend(&arg_ptrs);
    words.push(0);
    words.extend(&env_ptrs);
    words.push(0);
    for &(key, value) in aux {
        let value = match key {
            at::RANDOM => random,
            at::PLATFORM => platform,
            at::EXECFN => execfn,
            _ => value,
        };
        words.extend([key, value]);
    }
    words.extend([at::NULL, 0]);

    let mut sp = (sp - 8 * words.len() as u64) & !15;
    let start = sp;
    for w in words {
        // SAFETY: as above, inside the stack mapping and 8-byte aligned.
        unsafe { std::ptr::write(sp as *mut u64, w) };
        sp += 8;
    }
    start
}

/// The CPUID leaf 1 EDX bits, which Linux reports as `AT_HWCAP` on x86-64.
fn hwcap() -> u64 {
    #[cfg(target_arch = "x86_64")]
    {
        u64::from(std::arch::x86_64::__cpuid(1).edx)
    }
    #[cfg(not(target_arch = "x86_64"))]
    {
        0
    }
}

/// Starts the program and never returns: the guest ends the process with
/// `exit_group`, whose status becomes this process's.
pub fn exec(launch: &Launch) -> Result<Infallible, Error> {
    if !cfg!(target_arch = "x86_64") {
        return Err(Error::ForeignHost);
    }
    let ld_so = Image::map_program(&launch.ld_so, FsRewrite::On)?;
    let phdr = ld_so
        .program_headers()
        .ok_or_else(|| Error::BadPath(launch.ld_so.clone()))?;

    // A bootstrap control block: ld.so issues system calls, and reads the stack
    // guard, before it sets up its own thread pointer with arch_prctl.
    let bootstrap = vec![0u64; 512].into_boxed_slice();
    let tp = Box::leak(bootstrap).as_mut_ptr() as u64;
    let block = HostBlock::new(tp)?;
    // SAFETY: tp is a zeroed, leaked 4 KiB block that lives for the process.
    unsafe {
        std::ptr::write(tp as *mut u64, tp);
        std::ptr::write((tp + 0x10) as *mut u64, tp);
        attach(tp, &block);
    }
    syscall::BLOCK.with(|b| *b.borrow_mut() = Some(block));
    // The shims built with the rebuilt glibc, served at the paths their Linux
    // counterparts are loaded from.
    if let Some(dir) = launch.ld_so.parent() {
        let shim = dir.join("shims/steamclient.so");
        if let (Ok(mut slot), Ok(path)) = (
            steam::SHIM.lock(),
            CString::new(shim.to_string_lossy().as_bytes()),
        ) {
            *slot = Some(path);
        }
        let shim = dir.join("shims/libSDL3.so.0");
        if let (Ok(mut slot), Ok(path)) = (
            sdl::SHIM.lock(),
            CString::new(shim.to_string_lossy().as_bytes()),
        ) {
            *slot = Some(path);
        }
        let shim = dir.join("shims/libvulkan.so.1");
        if let (Ok(mut slot), Ok(path)) = (
            vulkan::SHIM.lock(),
            CString::new(shim.to_string_lossy().as_bytes()),
        ) {
            *slot = Some(path);
        }
    }
    if let Some(overlay) = &launch.overlay {
        paths::set_overlay(overlay.clone());
    }
    if let Ok(mut cmdline) = procfs::CMDLINE.lock() {
        for arg in std::iter::once(launch.program.to_string_lossy().into_owned())
            .chain(launch.args.iter().cloned())
        {
            cmdline.extend_from_slice(arg.as_bytes());
            cmdline.push(0);
        }
    }
    if let Ok(mut exe) = syscall::EXE.lock() {
        *exe = launch.program.to_string_lossy().into_owned();
    }
    set_syscall_handler(syscall::handle);

    // ld.so runs as the program and loads the real one: ld.so --library-path
    // DIRS PROGRAM ARGS.
    let lib_path = launch
        .library_path
        .iter()
        .map(|p| p.to_string_lossy())
        .collect::<Vec<_>>()
        .join(":");
    let mut argv = vec![
        c_string(&launch.ld_so.to_string_lossy(), &launch.ld_so)?,
        c_string("--library-path", &launch.ld_so)?,
        c_string(&lib_path, &launch.ld_so)?,
        c_string(&launch.program.to_string_lossy(), &launch.program)?,
    ];
    for a in &launch.args {
        argv.push(c_string(a, &launch.program)?);
    }
    let env = launch
        .env
        .iter()
        .map(|e| c_string(e, &launch.program))
        .collect::<Result<Vec<_>, _>>()?;

    let elf = ld_so.elf();
    let phnum = elf.segments.len() as u64;
    // SAFETY: getuid and friends have no preconditions.
    let (uid, euid, gid, egid) = unsafe {
        (
            libc::getuid(),
            libc::geteuid(),
            libc::getgid(),
            libc::getegid(),
        )
    };
    let aux = [
        (at::PHDR, phdr),
        (at::PHENT, 56),
        (at::PHNUM, phnum),
        (at::PAGESZ, 4096),
        (at::BASE, 0),
        (at::FLAGS, 0),
        (at::ENTRY, ld_so.address(elf.entry)),
        (at::UID, u64::from(uid)),
        (at::EUID, u64::from(euid)),
        (at::GID, u64::from(gid)),
        (at::EGID, u64::from(egid)),
        (at::SECURE, 0),
        (at::RANDOM, 0),
        (at::HWCAP, hwcap()),
        (at::HWCAP2, 0),
        (at::CLKTCK, 100),
        (at::PLATFORM, 0),
        (at::EXECFN, 0),
    ];

    // SAFETY: a fresh anonymous mapping for the guest's main stack.
    let stack = unsafe {
        libc::mmap(
            std::ptr::null_mut(),
            STACK_SIZE,
            libc::PROT_READ | libc::PROT_WRITE,
            libc::MAP_PRIVATE | libc::MAP_ANON,
            -1,
            0,
        )
    };
    if stack == libc::MAP_FAILED {
        return Err(Error::Loader(cs2metal_loader::Error::Map(
            std::io::Error::last_os_error(),
        )));
    }
    let top = stack as u64 + STACK_SIZE as u64;
    let sp = initial_stack(top, &argv, &env, &aux);
    let entry = ld_so.address(elf.entry);
    // ld.so stays mapped for the life of the process.
    std::mem::forget(ld_so);
    // SAFETY: tp is attached to this thread's host block, sp is a Linux initial
    // stack, and entry is ld.so's entry point.
    unsafe { enter_process(tp, entry, sp) }
}
