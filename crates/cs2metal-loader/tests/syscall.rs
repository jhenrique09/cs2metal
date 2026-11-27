//! The system call entry, with hand-assembled guest code that calls it as the
//! rebuilt glibc does and checks that every register a `syscall` preserves
//! survives, and that the entry leaves a frame record a stack walk can follow
//! from the handler into the guest.
#![cfg(target_arch = "x86_64")]

use std::sync::Mutex;

use cs2metal_loader::{Namespace, SyscallFrame, set_fast_syscall_handler, set_syscall_handler};

/// What the handler saw.
#[derive(Clone, Copy)]
struct Seen {
    /// The number and the six arguments.
    call: [u64; 7],
    /// The guest's rbx, rbp, and return address.
    saved: [u64; 3],
    /// Whether a frame-pointer walk from the handler reached the entry's frame
    /// record.
    linked: bool,
}

static SEEN: Mutex<Option<Seen>> = Mutex::new(None);

/// Whether the chain of frame pointers that starts at the caller's `rbp` reaches
/// `record` within a few frames, going up the stack only.
fn walk_reaches(record: u64) -> bool {
    let mut fp: u64;
    // SAFETY: reads a register.
    unsafe { core::arch::asm!("mov {}, rbp", out(reg) fp, options(nomem, nostack)) };
    for _ in 0..16 {
        if fp == record {
            return true;
        }
        if fp == 0 || fp % 8 != 0 || fp > record {
            return false;
        }
        // SAFETY: fp is a frame record on this thread's stack, below `record`.
        let next = unsafe { std::ptr::read(fp as *const u64) };
        if next <= fp {
            return false;
        }
        fp = next;
    }
    false
}

extern "C" fn handler(frame: &mut SyscallFrame) -> u64 {
    let f = *frame;
    let linked = walk_reaches(std::ptr::from_ref(&frame.saved_rbp) as u64);
    if let Ok(mut seen) = SEEN.lock() {
        *seen = Some(Seen {
            call: [f.number, f.arg0, f.arg1, f.arg2, f.arg3, f.arg4, f.arg5],
            saved: [f.saved_rbx, f.saved_rbp, f.return_address],
            linked,
        });
    }
    frame.number * 10
}

/// What the guest code holds in rbx and rbp across its system call.
const RBX: u32 = 0x8888;
const RBP: u32 = 0x7777;

const ARGS: [u64; 6] = [0x1111, 0x2222, 0x3333, 0x4444, 0x5555, 0x6666];

/// mov reg, imm32 for rdi, rsi, rdx, r10, r8, r9.
const MOV: [[u8; 3]; 6] = [
    [0x48, 0xc7, 0xc7],
    [0x48, 0xc7, 0xc6],
    [0x48, 0xc7, 0xc2],
    [0x49, 0xc7, 0xc2],
    [0x49, 0xc7, 0xc0],
    [0x49, 0xc7, 0xc1],
];
/// cmp reg, imm32 for the same registers.
const CMP: [[u8; 3]; 6] = [
    [0x48, 0x81, 0xff],
    [0x48, 0x81, 0xfe],
    [0x48, 0x81, 0xfa],
    [0x49, 0x81, 0xfa],
    [0x49, 0x81, 0xf8],
    [0x49, 0x81, 0xf9],
];

/// The guest code for system call `number`, and the offset of the instruction
/// after its system call.
fn guest_code(number: u32) -> (Vec<u8>, usize) {
    let mut c = vec![0x55, 0x53]; // push rbp; push rbx
    c.extend([0x48, 0xc7, 0xc5]); // mov rbp, imm32
    c.extend(RBP.to_le_bytes());
    c.extend([0x48, 0xc7, 0xc3]); // mov rbx, imm32
    c.extend(RBX.to_le_bytes());
    for (op, value) in MOV.iter().zip(ARGS) {
        c.extend(op);
        c.extend((value as u32).to_le_bytes());
    }
    c.push(0xb8); // mov eax, number
    c.extend(number.to_le_bytes());
    c.extend([0x48, 0x8d, 0x64, 0x24, 0x80]); // lea rsp, [rsp - 128]
    c.extend([0x65, 0xff, 0x14, 0x25, 0x20, 0, 0, 0]); // call qword ptr gs:[0x20]
    let resume = c.len();
    c.extend([0x48, 0x8d, 0xa4, 0x24, 0x80, 0, 0, 0]); // lea rsp, [rsp + 128]
    let mut jumps = Vec::new();
    for (op, value) in [([0x48, 0x81, 0xfd], RBP), ([0x48, 0x81, 0xfb], RBX)] {
        c.extend(op); // cmp rbp, imm32; cmp rbx, imm32
        c.extend(value.to_le_bytes());
        c.extend([0x75, 0]); // jne bad
        jumps.push(c.len());
    }
    for (op, value) in CMP.iter().zip(ARGS) {
        c.extend(op);
        c.extend((value as u32).to_le_bytes());
        c.extend([0x75, 0]); // jne bad
        jumps.push(c.len());
    }
    c.extend([0x5b, 0x5d, 0xc3]); // pop rbx; pop rbp; ret: rax holds the result
    let bad = c.len();
    for at in jumps {
        c[at - 1] = (bad - at) as u8;
    }
    c.extend([0x48, 0xc7, 0xc0, 0xff, 0xff, 0xff, 0xff]); // mov rax, -1
    c.extend([0x5b, 0x5d, 0xc3]); // pop rbx; pop rbp; ret
    (c, resume)
}

/// The number the fast handler answers; it declines every other.
const FAST: u32 = 777;

/// What the fast handler saw: the calls it answered, and the word at `gs:0x10`,
/// which in the guest domain is the guest's own thread pointer.
static FAST_SEEN: Mutex<(u32, u64)> = Mutex::new((0, 0));

extern "C" fn fast_handler(frame: &mut SyscallFrame) -> bool {
    if frame.number != u64::from(FAST) {
        return false;
    }
    let word: u64;
    // SAFETY: reads a word of the block GS points at, whichever domain this is.
    unsafe {
        core::arch::asm!(
            "mov {w}, qword ptr gs:[0x10]",
            w = out(reg) word,
            options(nostack, readonly, preserves_flags),
        );
    }
    if let Ok(mut seen) = FAST_SEEN.lock() {
        *seen = (seen.0 + 1, word);
    }
    frame.number = frame.arg0 + frame.arg5;
    true
}

/// Maps `code` as guest code and returns its address.
fn map_code(code: &[u8]) -> u64 {
    // SAFETY: a fresh anonymous mapping, written before it is made executable.
    unsafe {
        let p = libc::mmap(
            std::ptr::null_mut(),
            4096,
            libc::PROT_READ | libc::PROT_WRITE,
            libc::MAP_PRIVATE | libc::MAP_ANON,
            -1,
            0,
        );
        assert_ne!(p, libc::MAP_FAILED);
        std::ptr::copy_nonoverlapping(code.as_ptr(), p.cast::<u8>(), code.len());
        assert_eq!(
            libc::mprotect(p, 4096, libc::PROT_READ | libc::PROT_EXEC),
            0
        );
        p as u64
    }
}

#[test]
fn a_guest_system_call_reaches_the_handler_and_preserves_registers() {
    set_syscall_handler(handler);
    set_fast_syscall_handler(fast_handler);
    let (code, resume) = guest_code(39);
    let page = map_code(&code);
    let thread = Namespace::new().thread().expect("guest thread");
    // SAFETY: the code above takes no arguments and returns in rax.
    let result = unsafe { thread.call(page, &[]) }.expect("call");
    assert_eq!(
        result, 390,
        "the handler's result, and every register intact"
    );
    let seen = SEEN.lock().expect("lock").expect("handler ran");
    assert_eq!(
        seen.call,
        [39, 0x1111, 0x2222, 0x3333, 0x4444, 0x5555, 0x6666]
    );
    assert_eq!(
        seen.saved,
        [u64::from(RBX), u64::from(RBP), page + resume as u64],
        "the guest's rbx, rbp, and return address"
    );
    assert!(
        seen.linked,
        "the handler's frame pointers lead to the entry's frame record"
    );
}

/// A call the fast handler answers never leaves the guest domain: the handler
/// runs with GS at the guest's control block, the host handler is not asked, and
/// every register a `syscall` preserves survives just the same.
#[test]
fn a_call_the_fast_handler_answers_makes_no_gs_switch() {
    set_syscall_handler(handler);
    set_fast_syscall_handler(fast_handler);
    let (code, _) = guest_code(FAST);
    let page = map_code(&code);
    let thread = Namespace::new().thread().expect("guest thread");
    // SAFETY: the code takes no arguments and returns in rax.
    let result = unsafe { thread.call(page, &[]) }.expect("call");
    assert_eq!(result, 0x1111 + 0x6666, "the fast handler's result");
    let (calls, word) = *FAST_SEEN.lock().expect("lock");
    assert_eq!(calls, 1);
    assert_eq!(
        word,
        thread.thread_pointer(),
        "GS was at the guest's control block"
    );
}
