//! A report of the fault that ends the process.
//!
//! The guest's signal handlers are not installed yet, so a hardware fault in
//! guest code (a division by zero, a bad pointer) ends the process with nothing
//! said, and so does an abort in a host library the guest called. The handler here writes one line to standard error first: the signal,
//! the faulting instruction, the registers, and the return addresses the frame
//! pointers lead to. With `CS2METAL_MAPS=1` those addresses name guest modules.
//! It then restores the default action and returns, so the instruction faults
//! again and the process ends as it would have.
//!
//! The handler may run with GS at the guest's control block, where macOS's
//! thread data is out of reach: it uses its own stack buffer and `write` only.

/// The signals reported: SIGILL, SIGTRAP, SIGABRT, SIGFPE, SIGBUS, SIGSEGV.
const FATAL: [libc::c_int; 6] = [
    libc::SIGILL,
    libc::SIGTRAP,
    libc::SIGABRT,
    libc::SIGFPE,
    libc::SIGBUS,
    libc::SIGSEGV,
];

/// How many return addresses the report follows.
const FRAMES: usize = 32;

/// A line built without allocating.
struct Line {
    bytes: [u8; 1024],
    len: usize,
}

impl Line {
    const fn new() -> Self {
        Self {
            bytes: [0; 1024],
            len: 0,
        }
    }

    fn text(&mut self, text: &str) {
        for &b in text.as_bytes() {
            if self.len < self.bytes.len() {
                self.bytes[self.len] = b;
                self.len += 1;
            }
        }
    }

    fn hex(&mut self, value: u64) {
        self.text("0x");
        let digits = (16 - value.leading_zeros() as usize / 4).max(1);
        for i in (0..digits).rev() {
            let nibble = (value >> (4 * i)) & 0xf;
            let digit = b"0123456789abcdef"[nibble as usize];
            if self.len < self.bytes.len() {
                self.bytes[self.len] = digit;
                self.len += 1;
            }
        }
    }

    fn field(&mut self, name: &str, value: u64) {
        self.text(name);
        self.hex(value);
    }

    fn as_bytes(&self) -> &[u8] {
        &self.bytes[..self.len]
    }
}

/// The report's line for a fault: `registers` are rip, rsp, rbp, rax, rcx, rdx,
/// rbx, rsi, and rdi; `callers` the return addresses found.
fn describe(signal: u64, address: u64, registers: &[u64; 9], callers: &[u64]) -> Line {
    let mut line = Line::new();
    line.field("cs2metal: fault: signal ", signal);
    line.field(" at ", registers[0]);
    line.field(" address ", address);
    for (name, value) in [
        " rsp ", " rbp ", " rax ", " rcx ", " rdx ", " rbx ", " rsi ", " rdi ",
    ]
    .iter()
    .zip(&registers[1..])
    {
        line.field(name, *value);
    }
    line.text(" callers");
    for &caller in callers {
        line.field(" ", caller);
    }
    line.text("\n");
    line
}

/// The return addresses the frame pointers lead to, from `rbp`, while each
/// frame record lies above `rsp`, within the stack's first megabytes, and above
/// the one before it.
///
/// # Safety
///
/// `rsp` and `rbp` must be the registers of a thread whose stack is mapped
/// from `rsp` up to the records read.
unsafe fn callers(rsp: u64, rbp: u64, out: &mut [u64; FRAMES]) -> usize {
    const REACH: u64 = 8 << 20;
    let mut found = 0;
    let (mut below, mut frame) = (rsp, rbp);
    while found < out.len() && frame >= below && frame - rsp < REACH && frame % 8 == 0 {
        // SAFETY: a frame record on the faulting thread's stack, by the checks
        // above and the caller's contract.
        let [next, caller] = unsafe { std::ptr::read(frame as *const [u64; 2]) };
        if caller == 0 {
            break;
        }
        out[found] = caller;
        found += 1;
        below = frame + 16;
        frame = next;
    }
    found
}

#[cfg(target_arch = "x86_64")]
extern "C" fn report(signal: libc::c_int, info: *mut libc::siginfo_t, context: *mut libc::c_void) {
    // SAFETY: the kernel passes a siginfo_t and a ucontext_t to an SA_SIGINFO
    // handler; the machine context holds the interrupted thread's registers.
    let (address, registers) = unsafe {
        let state = &(*(*context.cast::<libc::ucontext_t>()).uc_mcontext).__ss;
        (
            (*info).si_addr as u64,
            [
                state.__rip,
                state.__rsp,
                state.__rbp,
                state.__rax,
                state.__rcx,
                state.__rdx,
                state.__rbx,
                state.__rsi,
                state.__rdi,
            ],
        )
    };
    let mut found = [0u64; FRAMES];
    // SAFETY: the interrupted thread's own stack and frame pointers.
    let count = unsafe { callers(registers[1], registers[2], &mut found) };
    let line = describe(signal as u64, address, &registers, &found[..count]);
    let bytes = line.as_bytes();
    // SAFETY: writes the buffer to standard error; restores the default action,
    // under which the faulting instruction, run again, ends the process.
    unsafe {
        libc::write(2, bytes.as_ptr().cast(), bytes.len());
        libc::signal(signal, libc::SIG_DFL);
    }
}

/// Installs the report for the fatal signals, on the stack of the thread that
/// faults.
pub(crate) fn install() {
    #[cfg(target_arch = "x86_64")]
    for signal in FATAL {
        // SAFETY: a zeroed sigaction with a handler of the SA_SIGINFO form.
        unsafe {
            let mut action: libc::sigaction = std::mem::zeroed();
            action.sa_sigaction = report as *const () as usize;
            action.sa_flags = libc::SA_SIGINFO;
            libc::sigemptyset(&mut action.sa_mask);
            libc::sigaction(signal, &action, std::ptr::null_mut());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_fault_is_described_in_one_line() {
        let registers = [0x1000, 0x7000, 0x7010, 1, 0, 3, 4, 5, 0xabcdef];
        let line = describe(8, 0x1000, &registers, &[0x2000, 0x3000]);
        assert_eq!(
            std::str::from_utf8(line.as_bytes()).expect("text"),
            "cs2metal: fault: signal 0x8 at 0x1000 address 0x1000 rsp 0x7000 rbp 0x7010 \
             rax 0x1 rcx 0x0 rdx 0x3 rbx 0x4 rsi 0x5 rdi 0xabcdef callers 0x2000 0x3000\n"
        );
    }

    #[test]
    fn callers_follow_the_frame_records_up_the_stack() {
        // Two frame records, then one that points back down: the walk stops.
        let mut stack = [0u64; 8];
        let base = stack.as_ptr() as u64;
        stack[0] = base + 32;
        stack[1] = 0x1111;
        stack[4] = base;
        stack[5] = 0x2222;
        // The walk reads the records through their addresses.
        std::hint::black_box(&mut stack);
        let mut found = [0u64; FRAMES];
        // SAFETY: the records lie in `stack`.
        let count = unsafe { callers(base, base, &mut found) };
        assert_eq!(found[..count], [0x1111, 0x2222]);
    }
}
