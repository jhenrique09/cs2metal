//! TLS, transform T-001, and the GS bridge, on the tlsprobe fixture. The expected
//! values are fixtures/expected/tlsprobe.txt, printed on Linux.
#![cfg(target_arch = "x86_64")]

use std::cell::Cell;
use std::path::{Path, PathBuf};

use cs2metal_elf::testing::ElfBuilder;
use cs2metal_loader::{Bridge, Error, FsRewrite, GuestThread, Import, Namespace, neutral};

fn fixture(name: &str) -> Option<PathBuf> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../target/fixtures")
        .join(name);
    if path.exists() {
        Some(path)
    } else {
        eprintln!("skipped: {name} not built; run tools/fixtures.sh");
        None
    }
}

thread_local! {
    static ECHOES: Cell<u32> = const { Cell::new(0) };
}

/// A host function that needs the host domain: it uses a Rust thread-local,
/// which macOS reaches through GS.
extern "C" fn host_echo(x: i32) -> i32 {
    ECHOES.with(|c| c.set(c.get() + 1));
    x * 3
}

fn load_probe(bridge: &mut Bridge) -> Option<Namespace> {
    let path = fixture("libtlsprobe.so")?;
    let mut ns = Namespace::new();
    // The library needs libc.so.6 only for __stack_chk_fail, which the bridge's
    // neutral functions provide.
    ns.rewrite_fs(FsRewrite::On).host_library("libc.so.6");
    ns.load(&path, &mut |import: Import<'_>| match import.name {
        "host_echo" => bridge.host(host_echo as *const () as u64).ok(),
        name => neutral(name),
    })
    .expect("load");
    Some(ns)
}

fn call(thread: &GuestThread, ns: &Namespace, name: &str, args: &[u64]) -> i32 {
    let f = ns.symbol(name).unwrap_or_else(|| panic!("{name}"));
    // SAFETY: every probe_* function takes int arguments and returns int; its
    // imports are bound to the neutral TLS functions and to host_echo's thunk.
    let value = unsafe { thread.call(f, args) }.expect("call");
    value as u32 as i32
}

#[test]
fn rewrites_every_fs_access_the_scan_proves() {
    let mut bridge = Bridge::new().expect("bridge");
    let Some(ns) = load_probe(&mut bridge) else {
        return;
    };
    let report = ns.modules()[0].image.fs_report().expect("T-001 ran");
    // fs:0 once, the canary ten times, initial-exec twice: as `cs2metal tls scan`.
    assert_eq!(report.rewritten, 13);
    assert_eq!(report.unexplained, 0);
}

#[test]
fn runs_tls_in_every_model_per_thread_with_the_bridge() {
    let mut bridge = Bridge::new().expect("bridge");
    let Some(ns) = load_probe(&mut bridge) else {
        return;
    };
    let expected = [(6009012, 7011015), (7011015, 9015021), (8013018, 11019027)];
    std::thread::scope(|scope| {
        for (id, (first, second)) in expected.into_iter().enumerate() {
            let ns = &ns;
            scope.spawn(move || {
                let thread = ns.thread().expect("guest thread");
                let by = id as u64 + 1;
                assert_eq!(call(&thread, ns, "probe_bump", &[by]), first);
                assert_eq!(call(&thread, ns, "probe_bump", &[by]), second);
                assert_eq!(call(&thread, ns, "probe_zero_sum", &[]), 2);
            });
        }
    });

    let main = ns.thread().expect("guest thread");
    assert_eq!(
        call(&main, &ns, "probe_bump", &[10]),
        15027039,
        "a fresh block"
    );
    assert_eq!(call(&main, &ns, "probe_zero_sum", &[]), 1);
    assert_eq!(call(&main, &ns, "probe_tp_is_self", &[]), 1);
    assert_eq!(call(&main, &ns, "probe_guarded", &[3]), 2208);
    let canary = main.control_word(0x28).expect("canary");
    assert_eq!(canary & 0xff, 0, "glibc's canary has a zero low byte");

    let before = ECHOES.with(Cell::get);
    assert_eq!(call(&main, &ns, "probe_call_host", &[5]), 16);
    assert_eq!(
        ECHOES.with(Cell::get),
        before + 1,
        "host_echo ran in the host domain"
    );
}

#[test]
fn refuses_unexplained_fs_decodes_unless_overridden() {
    let dir = tempfile::tempdir().expect("temp dir");
    // ret; then bytes no rule proves to be code that decode as mov eax, fs:[rax].
    let code = [0xc3, 0x64, 0x8b, 0x00];
    let lib = ElfBuilder::shared_object("libodd.so")
        .code(&code, &[(0, 1)])
        .build();
    let path = dir.path().join("libodd.so");
    std::fs::write(&path, lib).expect("write");

    let mut strict = Namespace::new();
    strict.rewrite_fs(FsRewrite::On);
    let refused = strict.load(&path, &mut |_| None);
    assert!(matches!(refused, Err(Error::UnexplainedFs(1))));

    let mut lenient = Namespace::new();
    lenient.rewrite_fs(FsRewrite::OnAllowingUnexplained);
    lenient.load(&path, &mut |_| None).expect("override");
    let report = lenient.modules()[0].image.fs_report().expect("T-001 ran");
    assert_eq!((report.rewritten, report.unexplained), (0, 1));
}
