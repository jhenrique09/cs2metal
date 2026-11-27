//! Guest proxies of host C++ objects: the generic proxy vtable and the fixing
//! thunk, which the Steam bridge uses for the client library's interfaces.
#![cfg(target_arch = "x86_64")]

use std::cell::Cell;
use std::sync::atomic::{AtomicU64, Ordering};

use cs2metal_loader::{Bridge, Namespace, set_return_fixup};

thread_local! {
    static HOST_CALLS: Cell<u32> = const { Cell::new(0) };
}

/// A host object: a vtable pointer, then a field.
#[repr(C)]
struct Object {
    vtable: *const [u64; 2],
    field: u64,
}

extern "C" fn add(this: *const Object, x: u64) -> u64 {
    HOST_CALLS.with(|c| c.set(c.get() + 1)); // only works in the host domain
    // SAFETY: the proxy passes the real object.
    unsafe { (*this).field + x }
}

static OTHER: AtomicU64 = AtomicU64::new(0);
static OTHER_PROXY: AtomicU64 = AtomicU64::new(0);

extern "C" fn other(_this: *const Object) -> u64 {
    OTHER.load(Ordering::Relaxed)
}

extern "C" fn fixup(value: u64) -> u64 {
    if value != 0 && value == OTHER.load(Ordering::Relaxed) {
        OTHER_PROXY.load(Ordering::Relaxed)
    } else {
        value
    }
}

#[test]
fn a_proxy_calls_the_real_method_in_the_host_domain_and_fixes_returns() {
    let vtable: &'static [u64; 2] = Box::leak(Box::new([
        add as *const () as u64,
        other as *const () as u64,
    ]));
    let object: &'static Object = Box::leak(Box::new(Object { vtable, field: 40 }));
    let second: &'static Object = Box::leak(Box::new(Object { vtable, field: 7 }));
    OTHER.store(std::ptr::from_ref(second) as u64, Ordering::Relaxed);

    let mut bridge = Bridge::new().expect("bridge");
    let proxy_vtable = bridge.proxy_vtable().expect("proxy vtable");
    let proxy: &'static [u64; 2] =
        Box::leak(Box::new([proxy_vtable, std::ptr::from_ref(object) as u64]));
    let second_proxy: &'static [u64; 2] =
        Box::leak(Box::new([proxy_vtable, OTHER.load(Ordering::Relaxed)]));
    OTHER_PROXY.store(std::ptr::from_ref(second_proxy) as u64, Ordering::Relaxed);
    set_return_fixup(fixup);

    // SAFETY: the proxy vtable is readable; its slots are the generic stubs.
    let slot = |i: usize| unsafe { *((proxy_vtable as *const u64).add(i)) };
    let thread = Namespace::new().thread().expect("guest thread");
    let this = std::ptr::from_ref(proxy) as u64;
    let before = HOST_CALLS.with(Cell::get);
    // SAFETY: slot 0 is add(this, x) behind the proxy, entered from the guest domain.
    let sum = unsafe { thread.call(slot(0), &[this, 2]) }.expect("call");
    assert_eq!(sum, 42, "the real object's field plus the argument");
    assert_eq!(
        HOST_CALLS.with(Cell::get),
        before + 1,
        "ran in the host domain"
    );
    // SAFETY: slot 1 is other(this), which returns a host object pointer.
    let returned = unsafe { thread.call(slot(1), &[this]) }.expect("call");
    assert_eq!(
        returned,
        std::ptr::from_ref(second_proxy) as u64,
        "returned as its proxy"
    );
    // SAFETY: the returned proxy works like the first.
    let again = unsafe { thread.call(slot(0), &[returned, 1]) }.expect("call");
    assert_eq!(again, 8);
}
