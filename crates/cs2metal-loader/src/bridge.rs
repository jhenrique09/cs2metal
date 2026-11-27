//! The GS bridge between the host domain and the guest domain.
//!
//! Under Rosetta 2 no `fs` base can be set, so guest code reaches its thread control
//! block through `gs` instead (transform T-001), and every thread switches its GS
//! base between the guest's block and the macOS TSD array with the machdep call
//! `thread_fast_set_cthread_self` (`0x3000003`), as the segment probe measured.
//!
//! - [`call_guest`] runs a guest function from host code: GS to the guest's block,
//!   the call, GS back to the host.
//! - A *host thunk* runs a host function from guest code: GS to the host, the call,
//!   GS back to the guest.
//! - A *guest thunk* is the reverse, for guest callbacks that host code calls: GS
//!   to the guest's block, the call, GS back to the host. Host code finds the
//!   thread's host block through a pthread TSD slot, since the guest's block is
//!   not at hand in the host domain. Its state lives on a small per-thread stack, not on the
//!   machine stack, so that arguments passed on the stack reach the host function
//!   where the ABI puts them.
//! - The *system call entry* is what the guest's rebuilt glibc calls through
//!   `tcbhead_t.sysinfo` instead of `syscall`: it preserves every register a Linux
//!   `syscall` preserves, switches to the host, and runs the registered handler.
//! - A *neutral* function touches no macOS thread data and is called directly.
//!
//! Every guest thread's control block points, in an area glibc 2.31 leaves unused,
//! at a *host block* that holds what the bridge needs for that thread: its host TSD
//! base, discovered at run time, the guest thread pointer to return to, and the
//! thunk's frames.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::error::{Error, Result};
use crate::memory::Region;

/// Offsets inside a guest thread control block, glibc 2.31's `tcbhead_t` on x86-64.
#[cfg_attr(not(target_arch = "x86_64"), allow(dead_code))]
pub(crate) mod tcb {
    pub const SELF: u64 = 0x0;
    pub const DTV: u64 = 0x8;
    pub const SELF_AGAIN: u64 = 0x10;
    /// `sysinfo`: the system call slot the rebuilt glibc calls through.
    pub const SYSINFO: u64 = 0x20;
    pub const STACK_GUARD: u64 = 0x28;
    pub const POINTER_GUARD: u64 = 0x30;
    /// The first word of `__glibc_unused2`, which glibc 2.31 never touches: the
    /// address of this thread's host block.
    pub const HOST_BLOCK: u64 = 0x80;
    /// Bytes the loader reserves for a control block it builds itself.
    pub const SIZE: u64 = 0x1000;
}

/// Offsets inside a host block.
#[cfg_attr(not(target_arch = "x86_64"), allow(dead_code))]
pub(crate) mod host {
    /// The thread's macOS TSD base.
    pub const TSD: u64 = 0x0;
    /// The guest thread pointer to restore when returning to the guest; a guest's
    /// `arch_prctl(ARCH_SET_FS)` changes it.
    pub const GUEST_TP: u64 = 0x8;
    /// The next free frame of the host thunk's stack.
    pub const BRIDGE_TOP: u64 = 0x10;
    /// The host stack pointer to return to when a guest thread ends.
    pub const SAVED_RSP: u64 = 0x18;
    /// The thunk's frames, 128 bytes each, to the end of the block: 31 nested
    /// guest-to-host calls per thread.
    pub const BRIDGE_FRAMES: u64 = 0x40;
    pub const BRIDGE_FRAME_SIZE: u64 = 128;
    pub const SIZE: u64 = 0x1000;
}

/// The registers of a guest system call, as the system call entry saves them.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct SyscallFrame {
    /// `rax`: the Linux system call number on entry.
    pub number: u64,
    pub arg5: u64,
    pub arg4: u64,
    pub arg3: u64,
    pub arg2: u64,
    pub arg1: u64,
    pub arg0: u64,
    pub saved_rbx: u64,
    /// The guest's `rbp`. With `return_address` after it, a frame record: the
    /// entry's `rbp` points here, so a frame-pointer walk of a thread inside a
    /// system call goes on into the guest code that made it.
    pub saved_rbp: u64,
    /// Where the guest resumes: the instruction after its `call` through the slot.
    pub return_address: u64,
}

/// Answers a guest system call in the host domain; the result goes to `rax`, a
/// negative errno for a failure, as on Linux.
pub type SyscallHandler = extern "C" fn(&mut SyscallFrame) -> u64;

static HANDLER: AtomicU64 = AtomicU64::new(0);
static FIXUP: AtomicU64 = AtomicU64::new(0);

/// Rewrites what a host function returns before the guest sees it, in the host
/// domain: the Steam bridge turns host interface pointers into guest proxies.
pub type ReturnFixup = extern "C" fn(u64) -> u64;

/// Installs the fixup that the fixing thunk ([`Bridge::host_fixed`]) applies.
pub fn set_return_fixup(fixup: ReturnFixup) {
    FIXUP.store(fixup as *const () as u64, Ordering::Release);
}

#[cfg_attr(not(target_arch = "x86_64"), allow(dead_code))]
extern "C" fn fixup(value: u64) -> u64 {
    let f = FIXUP.load(Ordering::Acquire);
    if f == 0 {
        return value;
    }
    // SAFETY: only set_return_fixup stores here, from a ReturnFixup.
    let f: ReturnFixup = unsafe { std::mem::transmute(f as usize) };
    f(value)
}

/// The byte offset, in every thread's host TSD array, of the slot that holds the
/// thread's host block: a pthread key's slot, made with the first host block. The
/// guest thunk reads it with GS at the host's TSD. Zero until then, and zero in a
/// thread without a host block.
#[cfg_attr(not(target_arch = "x86_64"), allow(dead_code))]
static BLOCK_SLOT: AtomicU64 = AtomicU64::new(0);

/// The pthread key whose TSD slot [`BLOCK_SLOT`] is, made once.
fn block_key() -> Option<libc::pthread_key_t> {
    static KEY: std::sync::OnceLock<Option<libc::pthread_key_t>> = std::sync::OnceLock::new();
    *KEY.get_or_init(|| {
        let mut key: libc::pthread_key_t = 0;
        // SAFETY: pthread_key_create writes a new key; no destructor.
        (unsafe { libc::pthread_key_create(&mut key, None) } == 0).then_some(key)
    })
}

/// Records `block` as the calling thread's host block in its TSD slot, checking
/// that the slot is where the guest thunk reads it.
fn register_block(block: u64) -> Result<()> {
    let key = block_key().ok_or(Error::ForeignHost)?;
    // SAFETY: setting this thread's value of a key this module made.
    if unsafe { libc::pthread_setspecific(key, block as *const libc::c_void) } != 0 {
        return Err(Error::ForeignHost);
    }
    // On macOS a key's value lives in TSD slot `key`; the thunk depends on that.
    if imp::tsd_word(key) != block {
        return Err(Error::ForeignHost);
    }
    BLOCK_SLOT.store(8 * key, Ordering::Release);
    Ok(())
}

/// Reached when host code calls a guest callback on a thread that has no host
/// block: there is no guest thread state to run it with.
#[cfg_attr(not(target_arch = "x86_64"), allow(dead_code))]
extern "C" fn foreign_callback(target: u64) -> ! {
    eprintln!("cs2metal: a host thread without guest state called the guest function {target:#x}");
    std::process::abort()
}

/// Installs the handler every guest system call reaches.
pub fn set_syscall_handler(handler: SyscallHandler) {
    HANDLER.store(handler as *const () as u64, Ordering::Release);
}

/// Called by the system call entry, in the host domain.
#[cfg_attr(not(target_arch = "x86_64"), allow(dead_code))]
extern "C" fn dispatch(frame: &mut SyscallFrame) -> u64 {
    let handler = HANDLER.load(Ordering::Acquire);
    if handler == 0 {
        const ENOSYS: u64 = 38;
        return ENOSYS.wrapping_neg();
    }
    // SAFETY: only set_syscall_handler stores here, from a SyscallHandler.
    let handler: SyscallHandler = unsafe { std::mem::transmute(handler as usize) };
    handler(frame)
}

#[cfg(target_arch = "x86_64")]
mod imp {
    use super::{host, tcb};

    // The host thunk. Entered from guest code with GS at the guest's block, the
    // arguments in the SysV registers and on the stack, and r11 holding the host
    // function, set by the stub that jumped here.
    //
    // Frame layout on the host block's stack: 0 rax (vector count of a variadic
    // call), 8 guest return address, 16 rbx, 24 r12, 32 target, 40 rcx, 48 rdi;
    // reused for rax and rdx on the way out. rbx holds the host block across the
    // call, r12 the frame.
    core::arch::global_asm!(
        ".p2align 4",
        ".globl _cs2metal_host_thunk",
        "_cs2metal_host_thunk:",
        "mov r10, qword ptr gs:[{block}]",
        "mov r10, qword ptr [r10 + {top}]",
        "mov qword ptr [r10 + 0], rax",
        "pop rax",
        "mov qword ptr [r10 + 8], rax",
        "mov qword ptr [r10 + 16], rbx",
        "mov qword ptr [r10 + 24], r12",
        "mov qword ptr [r10 + 32], r11",
        "mov qword ptr [r10 + 40], rcx",
        "mov qword ptr [r10 + 48], rdi",
        "mov rbx, qword ptr gs:[{block}]",
        "mov r12, r10",
        "lea r10, [r12 + {frame}]",
        "mov qword ptr [rbx + {top}], r10",
        "mov rdi, qword ptr [rbx + {tsd}]",
        "mov eax, 0x3000003",
        "syscall",
        "mov rdi, qword ptr [r12 + 48]",
        "mov rcx, qword ptr [r12 + 40]",
        "mov rax, qword ptr [r12 + 0]",
        "call qword ptr [r12 + 32]",
        "mov qword ptr [r12 + 40], rax",
        "mov qword ptr [r12 + 48], rdx",
        "mov rdi, qword ptr [rbx + {guest}]",
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
        block = const tcb::HOST_BLOCK,
        top = const host::BRIDGE_TOP,
        tsd = const host::TSD,
        guest = const host::GUEST_TP,
        frame = const host::BRIDGE_FRAME_SIZE,
    );

    // The fixing thunk: the host thunk, plus a call to the registered return
    // fixup on rax, in the host domain, with rdx, xmm0, and xmm1 kept. Frame
    // slots 64 and 80 hold xmm0 and xmm1.
    core::arch::global_asm!(
        ".p2align 4",
        ".globl _cs2metal_fixed_thunk",
        "_cs2metal_fixed_thunk:",
        "mov r10, qword ptr gs:[{block}]",
        "mov r10, qword ptr [r10 + {top}]",
        "mov qword ptr [r10 + 0], rax",
        "pop rax",
        "mov qword ptr [r10 + 8], rax",
        "mov qword ptr [r10 + 16], rbx",
        "mov qword ptr [r10 + 24], r12",
        "mov qword ptr [r10 + 32], r11",
        "mov qword ptr [r10 + 40], rcx",
        "mov qword ptr [r10 + 48], rdi",
        "mov rbx, qword ptr gs:[{block}]",
        "mov r12, r10",
        "lea r10, [r12 + {frame}]",
        "mov qword ptr [rbx + {top}], r10",
        "mov rdi, qword ptr [rbx + {tsd}]",
        "mov eax, 0x3000003",
        "syscall",
        "mov rdi, qword ptr [r12 + 48]",
        "mov rcx, qword ptr [r12 + 40]",
        "mov rax, qword ptr [r12 + 0]",
        "call qword ptr [r12 + 32]",
        "mov qword ptr [r12 + 48], rdx",
        "movdqu xmmword ptr [r12 + 64], xmm0",
        "movdqu xmmword ptr [r12 + 80], xmm1",
        "mov rdi, rax",
        "call {fixup}",
        "mov qword ptr [r12 + 40], rax",
        "movdqu xmm0, xmmword ptr [r12 + 64]",
        "movdqu xmm1, xmmword ptr [r12 + 80]",
        "mov rdi, qword ptr [rbx + {guest}]",
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
        block = const tcb::HOST_BLOCK,
        top = const host::BRIDGE_TOP,
        tsd = const host::TSD,
        guest = const host::GUEST_TP,
        frame = const host::BRIDGE_FRAME_SIZE,
        fixup = sym super::fixup,
    );

    // The guest thunk. Entered from host code with GS at the host's TSD, the
    // arguments in the SysV registers and on the stack, and r11 holding the guest
    // function, set by the stub that jumped here. The thread's host block comes
    // from its TSD slot; a thread without one (a host thread the guest never ran
    // on) cannot enter the guest. Same frame layout as the host thunk.
    core::arch::global_asm!(
        ".p2align 4",
        ".globl _cs2metal_guest_thunk",
        "_cs2metal_guest_thunk:",
        "mov r10, qword ptr [rip + {slot}]",
        "mov r10, qword ptr gs:[r10]",
        "test r10, r10",
        "jz 9f",
        "mov r10, qword ptr [r10 + {top}]",
        "mov qword ptr [r10 + 0], rax",
        "pop rax",
        "mov qword ptr [r10 + 8], rax",
        "mov qword ptr [r10 + 16], rbx",
        "mov qword ptr [r10 + 24], r12",
        "mov qword ptr [r10 + 32], r11",
        "mov qword ptr [r10 + 40], rcx",
        "mov qword ptr [r10 + 48], rdi",
        "mov r12, r10",
        "mov rbx, qword ptr [rip + {slot}]",
        "mov rbx, qword ptr gs:[rbx]",
        "lea r10, [r12 + {frame}]",
        "mov qword ptr [rbx + {top}], r10",
        "mov rdi, qword ptr [rbx + {guest}]",
        "mov eax, 0x3000003",
        "syscall",
        "mov rdi, qword ptr [r12 + 48]",
        "mov rcx, qword ptr [r12 + 40]",
        "mov rax, qword ptr [r12 + 0]",
        "call qword ptr [r12 + 32]",
        "mov qword ptr [r12 + 40], rax",
        "mov qword ptr [r12 + 48], rdx",
        "mov rdi, qword ptr [rbx + {tsd}]",
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
        "9:",
        "mov rdi, r11",
        "call {foreign}",
        "ud2",
        slot = sym super::BLOCK_SLOT,
        top = const host::BRIDGE_TOP,
        tsd = const host::TSD,
        guest = const host::GUEST_TP,
        frame = const host::BRIDGE_FRAME_SIZE,
        foreign = sym super::foreign_callback,
    );

    // The system call entry. Entered by `call *%gs:0x20` from the rebuilt glibc,
    // below the red zone, with rax the Linux number and rdi, rsi, rdx, r10, r8, r9
    // the arguments. Like `syscall`, it changes only rax (the result), rcx, and r11;
    // the x87, MMX, and SSE state is saved with fxsave around the host code.
    //
    // rbp is pushed first and then points at its own saved value, under the
    // return address: a standard frame record, so that a debugger or `sample`
    // walks from the host code answering the call into the guest code that made
    // it. The SyscallFrame is the 64 bytes below the record and those two words.
    core::arch::global_asm!(
        ".p2align 4",
        ".globl _cs2metal_syscall_entry",
        "_cs2metal_syscall_entry:",
        "push rbp",
        "mov rbp, rsp",
        "push rbx",
        "push rdi",
        "push rsi",
        "push rdx",
        "push r10",
        "push r8",
        "push r9",
        "push rax",
        "mov rbx, qword ptr gs:[{block}]",
        "mov rdi, qword ptr [rbx + {tsd}]",
        "mov eax, 0x3000003",
        "syscall",
        "lea rdi, [rbp - {below}]",
        "sub rsp, 576",
        "and rsp, -64",
        "fxsave64 [rsp]",
        "call {dispatch}",
        "mov qword ptr [rbp - {below}], rax",
        "fxrstor64 [rsp]",
        "lea rsp, [rbp - {below}]",
        "mov rdi, qword ptr [rbx + {guest}]",
        "mov eax, 0x3000003",
        "syscall",
        "pop rax",
        "pop r9",
        "pop r8",
        "pop r10",
        "pop rdx",
        "pop rsi",
        "pop rdi",
        "pop rbx",
        "pop rbp",
        "ret",
        block = const tcb::HOST_BLOCK,
        tsd = const host::TSD,
        guest = const host::GUEST_TP,
        below = const core::mem::offset_of!(super::SyscallFrame, saved_rbp),
        dispatch = sym super::dispatch,
    );

    // A guest thread made by clone: saves the host's callee-saved registers and
    // stack pointer in the host block, then enters the guest at rip with GS at
    // tp, rsp at the guest stack, and rax 0, as a new Linux thread returns from
    // clone. leave_guest_thread comes back here, on the host stack, with the
    // thread's exit status. Arguments: rdi tp, rsi rip, rdx rsp, rcx host block.
    core::arch::global_asm!(
        ".p2align 4",
        ".globl _cs2metal_run_guest_thread",
        "_cs2metal_run_guest_thread:",
        "push rbx",
        "push rbp",
        "push r12",
        "push r13",
        "push r14",
        "push r15",
        "mov qword ptr [rcx + {saved}], rsp",
        "mov r12, rsi",
        "mov r13, rdx",
        "mov eax, 0x3000003",
        "syscall",
        "mov rsp, r13",
        "xor eax, eax",
        "jmp r12",
        ".p2align 4",
        ".globl _cs2metal_leave_guest_thread",
        "_cs2metal_leave_guest_thread:",
        "mov rsp, qword ptr [rdi + {saved}]",
        "mov rax, rsi",
        "pop r15",
        "pop r14",
        "pop r13",
        "pop r12",
        "pop rbp",
        "pop rbx",
        "ret",
        saved = const host::SAVED_RSP,
    );

    unsafe extern "C" {
        fn cs2metal_host_thunk();
        fn cs2metal_fixed_thunk();
        fn cs2metal_guest_thunk();
        fn cs2metal_syscall_entry();
        pub(crate) fn cs2metal_run_guest_thread(tp: u64, rip: u64, rsp: u64, block: u64) -> u64;
        pub(crate) fn cs2metal_leave_guest_thread(block: u64, status: u64) -> !;
    }

    pub(crate) fn thunk_entry() -> u64 {
        cs2metal_host_thunk as *const () as u64
    }

    pub(crate) fn syscall_entry() -> u64 {
        cs2metal_syscall_entry as *const () as u64
    }

    pub(crate) fn fixed_thunk_entry() -> u64 {
        cs2metal_fixed_thunk as *const () as u64
    }

    pub(crate) fn guest_thunk_entry() -> u64 {
        cs2metal_guest_thunk as *const () as u64
    }

    pub(crate) fn tsd_word(index: u64) -> u64 {
        gs_word(index)
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
    /// `tp` must be a guest control block owned by the calling thread whose host
    /// block holds this thread's TSD base and `tp` itself as the guest pointer, and
    /// `f` guest code that takes these arguments.
    pub(crate) unsafe fn call_guest(tp: u64, f: u64, args: &[u64; 6]) -> u64 {
        let result: u64;
        // SAFETY: the caller's contract. The host block restores GS before any
        // host code runs; r12-r14 are callee-saved, so the guest preserves them.
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
                "mov rdi, qword ptr gs:[{block}]",
                "mov rdi, qword ptr [rdi + {tsd}]",
                "mov eax, 0x3000003",
                "syscall",
                "mov rax, r12",
                block = const tcb::HOST_BLOCK,
                tsd = const host::TSD,
                inout("r12") f => _,
                in("r13") args.as_ptr(),
                in("r14") tp,
                lateout("rax") result,
                clobber_abi("C"),
            );
        }
        result
    }

    /// Enters a guest program as the Linux kernel does: GS at `tp`, the stack
    /// pointer at the initial stack, `rdx` zero (no exit function), and a jump to
    /// `entry`. It never returns; the guest ends the process with `exit_group`.
    ///
    /// # Safety
    ///
    /// `tp` must be a control block of the calling thread with its host block set,
    /// `stack` a Linux initial stack (argc, argv, envp, auxv), and `entry` the
    /// program's entry point.
    pub(crate) unsafe fn enter_process(tp: u64, entry: u64, stack: u64) -> ! {
        // SAFETY: the caller's contract; nothing of the host's stack is used again.
        unsafe {
            core::arch::asm!(
                "mov rdi, r14",
                "mov eax, 0x3000003",
                "syscall",
                "mov rsp, r13",
                "xor edx, edx",
                "xor ebp, ebp",
                "jmp r12",
                in("r12") entry,
                in("r13") stack,
                in("r14") tp,
                options(noreturn),
            );
        }
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
    pub(crate) fn syscall_entry() -> u64 {
        0
    }
    pub(crate) fn fixed_thunk_entry() -> u64 {
        0
    }
    pub(crate) fn guest_thunk_entry() -> u64 {
        0
    }
    pub(crate) fn tsd_word(_index: u64) -> u64 {
        0
    }
    pub(crate) fn host_tsd_base() -> Option<u64> {
        None
    }
    pub(crate) unsafe fn call_guest(_tp: u64, _f: u64, _args: &[u64; 6]) -> u64 {
        0
    }
    pub(crate) unsafe fn enter_process(_tp: u64, _entry: u64, _stack: u64) -> ! {
        std::process::abort()
    }
    pub(crate) unsafe extern "C" fn tls_get_addr(_index: *const [u64; 2]) -> u64 {
        0
    }
    pub(crate) extern "C" fn stack_chk_fail() -> ! {
        std::process::abort()
    }
}

pub(crate) use imp::{call_guest, host_tsd_base};

/// The address the guest's system call slot (`tcbhead_t.sysinfo`) must hold.
pub fn syscall_entry() -> u64 {
    imp::syscall_entry()
}

/// Addresses of the neutral functions the loader itself provides to the guest.
pub fn neutral(name: &str) -> Option<u64> {
    match name {
        "__tls_get_addr" => Some(imp::tls_get_addr as *const () as u64),
        "__stack_chk_fail" => Some(imp::stack_chk_fail as *const () as u64),
        _ => None,
    }
}

/// The per-thread state the bridge keeps outside the guest's control block.
#[derive(Debug)]
pub struct HostBlock {
    region: Region,
}

impl HostBlock {
    /// A host block for the calling thread, returning to the guest at `guest_tp`.
    pub fn new(guest_tp: u64) -> Result<Self> {
        let tsd = host_tsd_base().ok_or(Error::ForeignHost)?;
        let region = Region::reserve(host::SIZE)?;
        region.map_zero(0, host::SIZE)?;
        let base = region.address();
        region.write_u64(host::TSD, tsd)?;
        region.write_u64(host::GUEST_TP, guest_tp)?;
        region.write_u64(host::BRIDGE_TOP, base + host::BRIDGE_FRAMES)?;
        register_block(base)?;
        Ok(Self { region })
    }

    pub fn address(&self) -> u64 {
        self.region.address()
    }

    /// The host function and guest return address of the innermost thunk call
    /// on this block's thread, for diagnostics.
    pub fn current_call(&self) -> Option<(u64, u64)> {
        let top = self.region.read_u64(host::BRIDGE_TOP).ok()?;
        let frame = top.checked_sub(host::BRIDGE_FRAME_SIZE)?;
        let offset = frame.checked_sub(self.region.address())?;
        if offset < host::BRIDGE_FRAMES {
            return None;
        }
        let target = self.region.read_u64(offset + 32).ok()?;
        let back = self.region.read_u64(offset + 8).ok()?;
        Some((target, back))
    }

    /// The guest thread pointer the bridge returns to.
    pub fn guest_tp(&self) -> u64 {
        self.region.read_u64(host::GUEST_TP).unwrap_or(0)
    }

    /// Changes the guest thread pointer the bridge returns to, as
    /// `arch_prctl(ARCH_SET_FS)` does.
    pub fn set_guest_tp(&self, tp: u64) -> Result<()> {
        self.region.write_u64(host::GUEST_TP, tp)
    }
}

/// What [`probe_neutral`] fills its control block with. As a pointer it is not
/// canonical, so code that follows one of these words faults.
const POISON: u64 = 0xdead_0000_0000_0000;

/// Runs `f(arg)` in the guest domain of a control block that holds no thread
/// data, to show that `f` is neutral: that it may be called from guest code
/// without a switch to the host domain.
///
/// GS points at a block of poison words for the call. macOS code that needs the
/// thread's data reads it through GS: `errno`, `pthread_self`, thread-local
/// variables, and every lock of libSystem. Reading a poison word and following it
/// stops the process, and a word written is reported as [`Error::NotNeutral`], so
/// a return with `Ok` says `f` did neither on the path it took. It says nothing of
/// paths it did not take, such as a library's error handling.
///
/// For tests and diagnosis: the calling thread's host block registration is put
/// back as it was, but the call must not be made while a guest thunk of this
/// thread is in progress.
pub fn probe_neutral(f: extern "C" fn(u64) -> u64, arg: u64) -> Result<u64> {
    if !cfg!(target_arch = "x86_64") {
        return Err(Error::ForeignHost);
    }
    let key = block_key().ok_or(Error::ForeignHost)?;
    // SAFETY: reading this thread's value of a key this module made.
    let registered = unsafe { libc::pthread_getspecific(key) };
    let region = Region::reserve(tcb::SIZE)?;
    region.map_zero(0, tcb::SIZE)?;
    let tp = region.address();
    let block = HostBlock::new(tp)?;
    let expected = |offset: u64| {
        if offset == tcb::HOST_BLOCK {
            block.address()
        } else {
            POISON | offset
        }
    };
    for offset in (0..tcb::SIZE).step_by(8) {
        region.write_u64(offset, expected(offset))?;
    }
    // SAFETY: tp is a control block of this thread whose host block holds this
    // thread's TSD base and tp as the guest pointer; f takes one integer.
    let value = unsafe { call_guest(tp, f as usize as u64, &[arg, 0, 0, 0, 0, 0]) };
    // SAFETY: putting this thread's value of the key back.
    unsafe { libc::pthread_setspecific(key, registered) };
    for offset in (0..tcb::SIZE).step_by(8) {
        if region.read_u64(offset)? != expected(offset) {
            return Err(Error::NotNeutral { offset });
        }
    }
    Ok(value)
}

/// Makes `tp` a control block the bridge can use: its system call slot and its
/// host block pointer. The block must be writable for 0x88 bytes.
///
/// # Safety
///
/// `tp` must point at a writable guest control block.
pub unsafe fn attach(tp: u64, block: &HostBlock) {
    // SAFETY: the caller's contract; both words lie inside tcbhead_t.
    unsafe {
        std::ptr::write_unaligned((tp + tcb::SYSINFO) as *mut u64, syscall_entry());
        std::ptr::write_unaligned((tp + tcb::HOST_BLOCK) as *mut u64, block.address());
    }
}

/// Enters a guest program; see the safety contract of the kernel-like start.
///
/// # Safety
///
/// `tp` must be attached to a host block of the calling thread, `stack` a Linux
/// initial stack, and `entry` the program's entry point. Never returns.
pub unsafe fn enter_process(tp: u64, entry: u64, stack: u64) -> ! {
    // SAFETY: the caller's contract.
    unsafe { imp::enter_process(tp, entry, stack) }
}

/// Runs a guest thread made by `clone` on the calling host thread until it calls
/// `exit`, and returns its exit status, back on the host stack.
///
/// # Safety
///
/// `tp` must be the new thread's control block, attached to `block`, which must
/// belong to the calling thread; `rip` and `rsp` are where the guest resumes, as
/// the kernel would resume a new thread returning from `clone`.
pub unsafe fn run_guest_thread(tp: u64, rip: u64, rsp: u64, block: &HostBlock) -> u64 {
    #[cfg(target_arch = "x86_64")]
    {
        // SAFETY: the caller's contract.
        unsafe { imp::cs2metal_run_guest_thread(tp, rip, rsp, block.address()) }
    }
    #[cfg(not(target_arch = "x86_64"))]
    {
        let _ = (tp, rip, rsp, block);
        0
    }
}

/// Ends the guest thread that `run_guest_thread` started on this host thread:
/// back to the host stack, where `run_guest_thread` returns `status`.
///
/// # Safety
///
/// Must be called in the host domain, from a system call of the guest thread
/// that `run_guest_thread` runs on this host thread with `block`.
pub unsafe fn leave_guest_thread(block: &HostBlock, status: u64) -> ! {
    #[cfg(target_arch = "x86_64")]
    {
        // SAFETY: the caller's contract.
        unsafe { imp::cs2metal_leave_guest_thread(block.address(), status) }
    }
    #[cfg(not(target_arch = "x86_64"))]
    {
        let _ = (block, status);
        std::process::abort()
    }
}

/// Generated stubs that enter host functions through the host thunk.
#[derive(Debug)]
pub struct Bridge {
    stubs: Region,
    used: u64,
    capacity: u64,
    by_target: HashMap<(u64, u64), u64>,
    proxy_vtable: Option<Region>,
}

/// Slots of the generic proxy vtable: more than any Steamworks interface has.
pub const PROXY_SLOTS: u64 = 256;

/// `movabs r11, target; movabs r10, thunk; jmp r10`.
const STUB_SIZE: u64 = 23;

impl Bridge {
    pub fn new() -> Result<Self> {
        if !cfg!(target_arch = "x86_64") {
            return Err(Error::ForeignHost);
        }
        let capacity = 256 * 1024;
        let stubs = Region::reserve(capacity)?;
        stubs.map_zero(0, capacity)?;
        stubs.protect(0, capacity, libc::PROT_READ | libc::PROT_EXEC)?;
        Ok(Self {
            stubs,
            used: 0,
            capacity,
            by_target: HashMap::new(),
            proxy_vtable: None,
        })
    }

    fn place(&mut self, code: &[u8]) -> Result<u64> {
        let len = code.len() as u64;
        if self.used + len > self.capacity {
            return Err(Error::Map(std::io::ErrorKind::OutOfMemory.into()));
        }
        let at = self.used;
        self.stubs
            .protect(0, self.capacity, libc::PROT_READ | libc::PROT_WRITE)?;
        self.stubs.write_bytes(at, code)?;
        self.stubs
            .protect(0, self.capacity, libc::PROT_READ | libc::PROT_EXEC)?;
        self.used += len;
        Ok(self.stubs.address() + at)
    }

    fn stub(&mut self, target: u64, entry: u64) -> Result<u64> {
        if let Some(&stub) = self.by_target.get(&(target, entry)) {
            return Ok(stub);
        }
        let mut code = Vec::with_capacity(STUB_SIZE as usize);
        code.extend([0x49, 0xbb]);
        code.extend(target.to_le_bytes());
        code.extend([0x49, 0xba]);
        code.extend(entry.to_le_bytes());
        code.extend([0x41, 0xff, 0xe2]);
        let stub = self.place(&code)?;
        self.by_target.insert((target, entry), stub);
        Ok(stub)
    }

    /// As [`Bridge::host`], with the registered return fixup applied to what the
    /// host function returns.
    pub fn host_fixed(&mut self, target: u64) -> Result<u64> {
        self.stub(target, imp::fixed_thunk_entry())
    }

    /// The vtable every guest proxy of a host C++ object uses. Slot `i` loads the
    /// real object from the proxy's second word, calls the real object's method
    /// `i` through the fixing thunk, and so proxies what that method returns too.
    pub fn proxy_vtable(&mut self) -> Result<u64> {
        if let Some(table) = &self.proxy_vtable {
            return Ok(table.address());
        }
        let table = Region::reserve(PROXY_SLOTS * 8)?;
        table.map_zero(0, PROXY_SLOTS * 8)?;
        for i in 0..PROXY_SLOTS {
            let mut code = Vec::with_capacity(27);
            code.extend([0x48, 0x8b, 0x7f, 0x08]); // mov rdi, [rdi + 8]: the real object
            code.extend([0x4c, 0x8b, 0x1f]); // mov r11, [rdi]: its vtable
            code.extend([0x4d, 0x8b, 0x9b]); // mov r11, [r11 + 8 * i]: its method
            code.extend(((8 * i) as u32).to_le_bytes());
            code.extend([0x49, 0xba]); // movabs r10, fixing thunk
            code.extend(imp::fixed_thunk_entry().to_le_bytes());
            code.extend([0x41, 0xff, 0xe2]); // jmp r10
            let stub = self.place(&code)?;
            table.write_u64(8 * i, stub)?;
        }
        table.protect(0, PROXY_SLOTS * 8, libc::PROT_READ)?;
        let address = table.address();
        self.proxy_vtable = Some(table);
        Ok(address)
    }

    /// The address guest code calls to reach the host function at `target` with a
    /// switch to the host domain: one stub per target, made on first request.
    pub fn host(&mut self, target: u64) -> Result<u64> {
        self.stub(target, imp::thunk_entry())
    }

    /// The address host code calls to reach the guest function at `target`, a
    /// callback the guest handed to a host library, with a switch to the guest
    /// domain: one stub per target, made on first request. It must be called on a
    /// thread that has a host block.
    pub fn guest(&mut self, target: u64) -> Result<u64> {
        self.stub(target, imp::guest_thunk_entry())
    }
}
