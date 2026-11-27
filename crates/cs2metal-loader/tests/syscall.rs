//! The system call entry, with hand-assembled guest code that calls it as the
//! rebuilt glibc does and checks that every register a `syscall` preserves
//! survives.
#![cfg(target_arch = "x86_64")]

use std::sync::Mutex;

use cs2metal_loader::{Namespace, SyscallFrame, set_syscall_handler};

static SEEN: Mutex<Option<[u64; 7]>> = Mutex::new(None);

extern "C" fn handler(frame: &mut SyscallFrame) -> u64 {
    let f = *frame;
    if let Ok(mut seen) = SEEN.lock() {
        *seen = Some([f.number, f.arg0, f.arg1, f.arg2, f.arg3, f.arg4, f.arg5]);
    }
    frame.number * 10
}

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

fn guest_code() -> Vec<u8> {
    let mut c = Vec::new();
    for (op, value) in MOV.iter().zip(ARGS) {
        c.extend(op);
        c.extend((value as u32).to_le_bytes());
    }
    c.extend([0xb8, 39, 0, 0, 0]); // mov eax, 39 (getpid)
    c.extend([0x48, 0x8d, 0x64, 0x24, 0x80]); // lea rsp, [rsp - 128]
    c.extend([0x65, 0xff, 0x14, 0x25, 0x20, 0, 0, 0]); // call qword ptr gs:[0x20]
    c.extend([0x48, 0x8d, 0xa4, 0x24, 0x80, 0, 0, 0]); // lea rsp, [rsp + 128]
    let mut jumps = Vec::new();
    for (op, value) in CMP.iter().zip(ARGS) {
        c.extend(op);
        c.extend((value as u32).to_le_bytes());
        c.extend([0x75, 0]); // jne bad
        jumps.push(c.len());
    }
    c.push(0xc3); // ret: rax holds the handler's result
    let bad = c.len();
    for at in jumps {
        c[at - 1] = (bad - at) as u8;
    }
    c.extend([0x48, 0xc7, 0xc0, 0xff, 0xff, 0xff, 0xff, 0xc3]); // mov rax, -1; ret
    c
}

#[test]
fn a_guest_system_call_reaches_the_handler_and_preserves_registers() {
    set_syscall_handler(handler);
    let code = guest_code();
    // SAFETY: a fresh anonymous mapping, written before it is made executable.
    let page = unsafe {
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
    };
    let thread = Namespace::new().thread().expect("guest thread");
    // SAFETY: the code above takes no arguments and returns in rax.
    let result = unsafe { thread.call(page, &[]) }.expect("call");
    assert_eq!(
        result, 390,
        "the handler's result, and every register intact"
    );
    let seen = SEEN.lock().expect("lock").expect("handler ran");
    assert_eq!(seen, [39, 0x1111, 0x2222, 0x3333, 0x4444, 0x5555, 0x6666]);
}
