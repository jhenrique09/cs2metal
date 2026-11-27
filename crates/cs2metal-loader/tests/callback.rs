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
