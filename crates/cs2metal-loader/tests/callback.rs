//! Guest callbacks called by host code: the guest thunk, which a host library
//! reaches when it calls a function pointer the guest gave it.
#![cfg(target_arch = "x86_64")]

use std::cell::Cell;
use std::sync::atomic::{AtomicU64, Ordering};

use cs2metal_loader::{Bridge, Namespace};

thread_local! {
    static HOST_CALLS: Cell<u32> = const { Cell::new(0) };
}

static CALLBACK: AtomicU64 = AtomicU64::new(0);
static SEEN_GS: AtomicU64 = AtomicU64::new(0);

/// The guest callback: eight arguments, two of them on the stack. It records the
/// first word GS points at, which in the guest domain is the control block's
/// self pointer.
extern "C" fn callback(a: u64, b: u64, c: u64, d: u64, e: u64, f: u64, g: u64, h: u64) -> u64 {
    let gs0: u64;
    // SAFETY: reads the first word of whatever block GS points at; both domains
    // have one.
    unsafe {
        core::arch::asm!("mov {}, qword ptr gs:[0]", out(reg) gs0, options(nostack, readonly));
    }
    SEEN_GS.store(gs0, Ordering::Relaxed);
    a + 2 * b + 3 * c + 4 * d + 5 * e + 6 * f + 7 * g + 8 * h
}

type Callback = extern "C" fn(u64, u64, u64, u64, u64, u64, u64, u64) -> u64;

/// The host library function, entered from the guest through the host thunk. It
/// calls the guest's callback through its guest stub, then checks that it is
/// back in the host domain.
extern "C" fn host_library(x: u64) -> u64 {
    HOST_CALLS.with(|c| c.set(c.get() + 1)); // only works in the host domain
    // SAFETY: CALLBACK holds the guest stub of `callback`, which has its type.
    let callback: Callback = unsafe { std::mem::transmute(CALLBACK.load(Ordering::Relaxed)) };
    let result = callback(x, 1, 1, 1, 1, 1, 1, 10);
    HOST_CALLS.with(|c| c.set(c.get() + 1));
    result
}

#[test]
fn host_code_calls_a_guest_callback_in_the_guest_domain() {
    let thread = Namespace::new().thread().expect("guest thread");
    let mut bridge = Bridge::new().expect("bridge");
    let into_host = bridge
        .host(host_library as *const () as u64)
        .expect("host stub");
    let into_guest = bridge
        .guest(callback as *const () as u64)
        .expect("guest stub");
    CALLBACK.store(into_guest, Ordering::Relaxed);

    let before = HOST_CALLS.with(Cell::get);
    // SAFETY: the stub enters host_library(x) from the guest domain.
    let got = unsafe { thread.call(into_host, &[5]) }.expect("call");
    assert_eq!(
        got,
        5 + 2 + 3 + 4 + 5 + 6 + 7 + 80,
        "every argument arrived"
    );
    assert_eq!(
        SEEN_GS.load(Ordering::Relaxed),
        thread.thread_pointer(),
        "the callback ran with GS at the guest's block"
    );
    assert_eq!(
        HOST_CALLS.with(Cell::get),
        before + 2,
        "the host library ran in the host domain before and after"
    );
    // A second call through the same stubs, nested state unwound.
    // SAFETY: as above.
    let again = unsafe { thread.call(into_host, &[0]) }.expect("call");
    assert_eq!(again, got - 5);
}

static COUNTED_CALLS: AtomicU64 = AtomicU64::new(0);

extern "C" fn sum(a: u64, b: u64, c: u64, d: u64, e: u64, f: u64) -> u64 {
    a + 2 * b + 3 * c + 4 * d + 5 * e + 6 * f
}

/// A counted entry adds one to its word on each call and passes every argument
/// on to the entry behind it.
#[test]
fn a_counted_entry_counts_and_passes_the_call_on() {
    let thread = Namespace::new().thread().expect("guest thread");
    let mut bridge = Bridge::new().expect("bridge");
    let into_host = bridge.host(sum as *const () as u64).expect("host stub");
    let counter = std::ptr::from_ref(&COUNTED_CALLS) as u64;
    let counted = bridge.counted(into_host, counter).expect("counted stub");
    for round in 1..=3u64 {
        // SAFETY: the stub counts, then enters sum from the guest domain.
        let got = unsafe { thread.call(counted, &[1, 2, 3, 4, 5, round]) }.expect("call");
        assert_eq!(got, 1 + 4 + 9 + 16 + 25 + 6 * round);
        assert_eq!(COUNTED_CALLS.load(Ordering::Relaxed), round);
    }
}

static BEFORE_SAW: AtomicU64 = AtomicU64::new(0);

/// Called before the entry: it records its argument and then ruins every
/// register a call may ruin, the vector ones too.
extern "C" fn before(first: u64) {
    BEFORE_SAW.store(first, Ordering::Relaxed);
    // SAFETY: clobbers only registers the C ABI lets a function change, all
    // declared.
    unsafe {
        core::arch::asm!(
            "pxor xmm0, xmm0", "pxor xmm1, xmm1", "pxor xmm2, xmm2", "pxor xmm3, xmm3",
            "pxor xmm4, xmm4", "pxor xmm5, xmm5", "pxor xmm6, xmm6", "pxor xmm7, xmm7",
            "mov rdi, -1", "mov rsi, -1", "mov rdx, -1", "mov rcx, -1", "mov r8, -1",
            "mov r9, -1", "mov rax, -1",
            out("xmm0") _, out("xmm1") _, out("xmm2") _, out("xmm3") _,
            out("xmm4") _, out("xmm5") _, out("xmm6") _, out("xmm7") _,
            out("rdi") _, out("rsi") _, out("rdx") _, out("rcx") _, out("r8") _,
            out("r9") _, out("rax") _,
            options(nostack),
        );
    }
}

/// Six integers, eight floats, and two integers on the stack.
#[allow(clippy::too_many_arguments)]
extern "C" fn mixed(
    a: u64,
    b: u64,
    c: u64,
    d: u64,
    e: u64,
    f: u64,
    x0: f64,
    x1: f64,
    x2: f64,
    x3: f64,
    x4: f64,
    x5: f64,
    x6: f64,
    x7: f64,
    g: u64,
    h: u64,
) -> f64 {
    (a + 2 * b + 3 * c + 4 * d + 5 * e + 6 * f + 7 * g + 8 * h) as f64
        + x0
        + 2.0 * x1
        + 3.0 * x2
        + 4.0 * x3
        + 5.0 * x4
        + 6.0 * x5
        + 7.0 * x6
        + 8.0 * x7
}

type Mixed = extern "C" fn(
    u64,
    u64,
    u64,
    u64,
    u64,
    u64,
    f64,
    f64,
    f64,
    f64,
    f64,
    f64,
    f64,
    f64,
    u64,
    u64,
) -> f64;

/// A preceded entry calls its function with the first argument, and the entry
/// behind it then sees every argument untouched: in the integer registers, in
/// the vector registers, and on the stack.
#[test]
fn a_preceded_entry_keeps_every_argument() {
    let mut bridge = Bridge::new().expect("bridge");
    let stub = bridge
        .preceded(before as *const () as u64, mixed as *const () as u64)
        .expect("preceded stub");
    // SAFETY: the stub runs `before` and jumps to `mixed`, whose type this is.
    let through: Mixed = unsafe { std::mem::transmute(stub as usize) };
    let got = through(
        11, 2, 3, 4, 5, 6, 0.5, 1.5, 2.5, 3.5, 4.5, 5.5, 6.5, 7.5, 7, 8,
    );
    let plain = mixed(
        11, 2, 3, 4, 5, 6, 0.5, 1.5, 2.5, 3.5, 4.5, 5.5, 6.5, 7.5, 7, 8,
    );
    assert_eq!(got, plain);
    assert_eq!(BEFORE_SAW.load(Ordering::Relaxed), 11);
}
