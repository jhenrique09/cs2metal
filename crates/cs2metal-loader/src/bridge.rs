//! The GS bridge between the host domain and the guest domain.
//!
//! Under Rosetta 2 no `fs` base can be set, so guest code reaches its thread control
//! block through `gs` instead (transform T-001), and every thread switches its GS
//! base between the guest's block and the macOS TSD array with the machdep call
//! `thread_fast_set_cthread_self` (`0x3000003`), as the
//! [segment probe](../../../docs/ROSETTA_SEGMENT_PROBE_2026-11-26.md) measured.
//!
//! - [`call_guest`] runs a guest function from host code: GS to the guest's block,
//!   the call, GS back to the host.
//! - A *host thunk* runs a host function from guest code: GS to the host, the call,
//!   GS back to the guest. Its state lives on a small per-thread stack inside the
//!   guest's control block, not on the machine stack, so that arguments passed on
//!   the stack reach the host function where the ABI puts them.
//! - A *neutral* function touches no macOS thread data and is called directly.
//!
//! The host TSD base of each thread is discovered at run time, never assumed.

use std::collections::HashMap;

use crate::error::{Error, Result};
use crate::memory::Region;

/// Offsets inside a guest thread control block. 0 to 0x30 are glibc's `tcbhead_t`;
/// the rest of the first page is cs2metal's own. Only the x86-64 assembly uses
/// some of them.
#[cfg_attr(not(target_arch = "x86_64"), allow(dead_code))]
pub(crate) mod tcb {
    pub const SELF: u64 = 0x0;
    pub const DTV: u64 = 0x8;
    pub const SELF_AGAIN: u64 = 0x10;
    pub const STACK_GUARD: u64 = 0x28;
    pub const POINTER_GUARD: u64 = 0x30;
    /// The host TSD base of the thread that owns the block.
    pub const HOST_TSD: u64 = 0x800;
    /// The next free frame of the host thunk's per-thread stack.
    pub const BRIDGE_TOP: u64 = 0x808;
    /// The thunk's frames, 64 bytes each, from here to the end of the page:
    /// 24 nested guest-to-host calls per thread.
    pub const BRIDGE_FRAMES: u64 = 0x900;
    pub const BRIDGE_FRAME_SIZE: u64 = 64;
    /// Bytes reserved for the control block, before the DTV.
    pub const SIZE: u64 = 0x1000;
}

#[cfg(target_arch = "x86_64")]
mod imp {
    use super::tcb;

    // The host thunk. Entered from guest code with GS at the guest's block, the
    // arguments in the SysV registers and on the stack, and r11 holding the host
    // function, set by the stub that jumped here.
    //
    // Frame layout on the per-thread bridge stack: 0 rax (vector count of a
    // variadic call), 8 guest return address, 16 rbx, 24 r12, 32 target, 40 rcx,
    // 48 rdi; reused for rax and rdx on the way out.
    core::arch::global_asm!(
        ".p2align 4",
        ".globl _cs2metal_host_thunk",
        "_cs2metal_host_thunk:",
        "mov r10, qword ptr gs:[{self_}]",
        "mov r10, qword ptr [r10 + {top}]",
        "mov qword ptr [r10 + 0], rax",
        "pop rax",
        "mov qword ptr [r10 + 8], rax",
        "mov qword ptr [r10 + 16], rbx",
        "mov qword ptr [r10 + 24], r12",
        "mov qword ptr [r10 + 32], r11",
        "mov qword ptr [r10 + 40], rcx",
        "mov qword ptr [r10 + 48], rdi",
        "mov rbx, qword ptr gs:[{self_}]",
        "mov r12, r10",
        "lea r10, [r12 + {frame}]",
        "mov qword ptr [rbx + {top}], r10",
        "mov rdi, qword ptr [rbx + {host}]",
        "mov eax, 0x3000003",
        "syscall",
        "mov rdi, qword ptr [r12 + 48]",
        "mov rcx, qword ptr [r12 + 40]",
        "mov rax, qword ptr [r12 + 0]",
        "call qword ptr [r12 + 32]",
        "mov qword ptr [r12 + 40], rax",
        "mov qword ptr [r12 + 48], rdx",
        "mov rdi, rbx",
        "mov eax, 0x3000003",
        "syscall",
        "mov rax, qword ptr [r12 + 40]",
        "mov rdx, qword ptr [r12 + 48]",
        "mov qword ptr [rbx + {top}], r12",
        "mov rcx, qword ptr [r12 + 8]",
        "mov rbx, qword ptr [r12 + 16]",
        "mov r12, qword ptr [r12 + 24]",
        "push rcx",
        "ret",
        self_ = const tcb::SELF,
        top = const tcb::BRIDGE_TOP,
        host = const tcb::HOST_TSD,
        frame = const tcb::BRIDGE_FRAME_SIZE,
    );

    unsafe extern "C" {
        fn cs2metal_host_thunk();
    }

    pub(crate) fn thunk_entry() -> u64 {
        cs2metal_host_thunk as *const () as u64
    }

    fn gs_word(index: u64) -> u64 {
        let value: u64;
        // SAFETY: reads the calling thread's own TSD array, in the host domain; the
        // first twelve slots always exist.
        unsafe {
            core::arch::asm!(
                "mov {v}, qword ptr gs:[{i} * 8]",
                v = out(reg) value,
                i = in(reg) index,
                options(nostack, readonly, preserves_flags),
            );
        }
        value
    }

    /// The calling thread's host TSD base: the address inside its pthread
    /// structure whose first twelve words equal `gs:[0..96]`.
    pub(crate) fn host_tsd_base() -> Option<u64> {
        let want: Vec<u64> = (0..12).map(gs_word).collect();
        // SAFETY: pthread_self has no preconditions.
        let me = unsafe { libc::pthread_self() } as usize;
        (0..0x400usize)
            .step_by(8)
            .map(|o| me + o)
            .find(|&at| {
                want.iter().enumerate().all(|(i, w)| {
                    // SAFETY: the pthread structure of a live thread is larger than
                    // 0x460 bytes and readable; this reads inside it.
                    unsafe { std::ptr::read((at + 8 * i) as *const u64) == *w }
                })
            })
            .map(|at| at as u64)
    }

    /// Calls the guest function `f` with up to six integer arguments, with GS at
    /// the control block `tp` for the duration of the call.
    ///
    /// # Safety
    ///
    /// `tp` must be a guest control block owned by the calling thread, with its
    /// host TSD slot set to this thread's base, and `f` guest code that takes these
    /// arguments.
    pub(crate) unsafe fn call_guest(tp: u64, f: u64, args: &[u64; 6]) -> u64 {
        let result: u64;
        // SAFETY: the caller's contract. The block's host slot restores GS before
        // any host code runs; r12-r14 are callee-saved, so the guest preserves them.
        unsafe {
            core::arch::asm!(
                "mov rdi, r14",
                "mov eax, 0x3000003",
                "syscall",
                "mov rdi, qword ptr [r13]",
                "mov rsi, qword ptr [r13 + 8]",
                "mov rdx, qword ptr [r13 + 16]",
                "mov rcx, qword ptr [r13 + 24]",
                "mov r8, qword ptr [r13 + 32]",
                "mov r9, qword ptr [r13 + 40]",
                "xor eax, eax",
                "call r12",
                "mov r12, rax",
                "mov rdi, qword ptr gs:[{host}]",
                "mov eax, 0x3000003",
                "syscall",
                "mov rax, r12",
                host = const tcb::HOST_TSD,
                inout("r12") f => _,
                in("r13") args.as_ptr(),
                in("r14") tp,
                lateout("rax") result,
                clobber_abi("C"),
            );
        }
        result
    }

    /// `__tls_get_addr`, neutral: it runs in the guest domain, reads the DTV of the
    /// block GS points at, and touches nothing else.
    ///
    /// # Safety
    ///
    /// Called by guest code with GS at a block built by this crate and `index`
    /// pointing at a `tls_index` the loader filled in.
    pub(crate) unsafe extern "C" fn tls_get_addr(index: *const [u64; 2]) -> u64 {
        let dtv: u64;
        // SAFETY: in the guest domain gs:[8] is the block's DTV pointer.
        unsafe {
            core::arch::asm!(
                "mov {d}, qword ptr gs:[{off}]",
                d = out(reg) dtv,
                off = const tcb::DTV,
                options(nostack, readonly, preserves_flags),
            );
        }
        // SAFETY: the loader wrote the module id and offset; the DTV has an entry
        // for every module with TLS, 16 bytes each, pointer first.
        unsafe {
            let [module, offset] = *index;
            let block = *((dtv + 16 * module) as *const u64);
            block.wrapping_add(offset)
        }
    }

    /// `__stack_chk_fail`, neutral: the stack protector found a smashed canary.
    pub(crate) extern "C" fn stack_chk_fail() -> ! {
        // SAFETY: ud2 raises SIGILL, the safest stop available in the guest domain.
        unsafe { core::arch::asm!("ud2", options(noreturn, nostack)) }
    }
}

#[cfg(not(target_arch = "x86_64"))]
mod imp {
    pub(crate) fn thunk_entry() -> u64 {
        0
    }
    pub(crate) fn host_tsd_base() -> Option<u64> {
        None
    }
    pub(crate) unsafe fn call_guest(_tp: u64, _f: u64, _args: &[u64; 6]) -> u64 {
        0
    }
    pub(crate) unsafe extern "C" fn tls_get_addr(_index: *const [u64; 2]) -> u64 {
        0
    }
    pub(crate) extern "C" fn stack_chk_fail() -> ! {
        std::process::abort()
    }
}

#[allow(unused_imports)]
pub(crate) use imp::{call_guest, host_tsd_base};

/// Addresses of the neutral functions the loader itself provides to the guest.
pub fn neutral(name: &str) -> Option<u64> {
    match name {
        "__tls_get_addr" => Some(imp::tls_get_addr as *const () as u64),
        "__stack_chk_fail" => Some(imp::stack_chk_fail as *const () as u64),
        _ => None,
    }
}

/// Generated stubs that enter host functions through the host thunk.
#[derive(Debug)]
pub struct Bridge {
    stubs: Region,
    used: u64,
    capacity: u64,
    by_target: HashMap<u64, u64>,
}

/// `movabs r11, target; movabs r10, thunk; jmp r10`.
const STUB_SIZE: u64 = 23;

impl Bridge {
    pub fn new() -> Result<Self> {
        if !cfg!(target_arch = "x86_64") {
            return Err(Error::ForeignHost);
        }
        let capacity = 64 * 1024;
        let stubs = Region::reserve(capacity)?;
        stubs.map_zero(0, capacity)?;
        stubs.protect(0, capacity, libc::PROT_READ | libc::PROT_EXEC)?;
        Ok(Self {
            stubs,
            used: 0,
            capacity,
            by_target: HashMap::new(),
        })
    }

    /// The address guest code calls to reach the host function at `target` with a
    /// switch to the host domain: one stub per target, made on first request.
    pub fn host(&mut self, target: u64) -> Result<u64> {
        if let Some(&stub) = self.by_target.get(&target) {
            return Ok(stub);
        }
        if self.used + STUB_SIZE > self.capacity {
            return Err(Error::Map(std::io::ErrorKind::OutOfMemory.into()));
        }
        let mut code = Vec::with_capacity(STUB_SIZE as usize);
        code.extend([0x49, 0xbb]);
        code.extend(target.to_le_bytes());
        code.extend([0x49, 0xba]);
        code.extend(imp::thunk_entry().to_le_bytes());
        code.extend([0x41, 0xff, 0xe2]);
        let at = self.used;
        self.stubs
            .protect(0, self.capacity, libc::PROT_READ | libc::PROT_WRITE)?;
        self.stubs.write_bytes(at, &code)?;
        self.stubs
            .protect(0, self.capacity, libc::PROT_READ | libc::PROT_EXEC)?;
        self.used += STUB_SIZE;
        let stub = self.stubs.address() + at;
        self.by_target.insert(target, stub);
        Ok(stub)
    }
}
