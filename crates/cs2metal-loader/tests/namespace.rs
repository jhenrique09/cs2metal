use std::path::{Path, PathBuf};

use cs2metal_loader::{Error, Import, Namespace};

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

fn nothing(_: Import<'_>) -> Option<u64> {
    None
}

/// The expected values are fixtures/expected/scope.txt, printed on Linux.
#[cfg(target_arch = "x86_64")]
#[test]
fn binds_like_glibc_across_modules() {
    let Some(path) = fixture("libscope_user.so") else {
        return;
    };
    let mut ns = Namespace::new();
    ns.load(&path, &mut nothing).expect("load");
    let names: Vec<_> = ns.modules().iter().map(|m| m.name.as_str()).collect();
    assert_eq!(
        names,
        ["libscope_user.so", "libscope_over.so", "libscope_base.so"],
        "breadth-first, in DT_NEEDED order, found through $ORIGIN"
    );
    // SAFETY: the scope libraries use no C library and no TLS.
    unsafe { ns.run_initializers() }.expect("initializers");

    let call = |name: &str| -> i32 {
        let address = ns.symbol(name).unwrap_or_else(|| panic!("{name}"));
        // SAFETY: every user_* function of the fixture takes nothing and returns int.
        let f = unsafe { std::mem::transmute::<usize, extern "C" fn() -> i32>(address as usize) };
        f()
    };
    assert_eq!(call("user_versioned"), 20, "default version SCOPE_2");
    assert_eq!(call("user_shared"), 2000, "first definition in scope order");
    assert_eq!(
        call("user_overridable"),
        200,
        "a strong definition earlier in scope"
    );
    assert_eq!(
        call("user_base_calls_shared"),
        2000,
        "interposed even inside the defining module"
    );
    assert_eq!(call("user_base_value"), 1);
    assert_eq!(call("user_absent_is_null"), 1, "weak reference left null");
    // SAFETY: as above.
    unsafe { ns.run_finalizers() }.expect("finalizers");
}

#[cfg(target_arch = "x86_64")]
#[test]
fn looks_up_hidden_and_default_versions() {
    let Some(path) = fixture("libscope_base.so") else {
        return;
    };
    let mut ns = Namespace::new();
    ns.load(&path, &mut nothing).expect("load");
    let version = |v: Option<&str>| {
        ns.lookup("versioned", v)
            .and_then(|(_, s)| s.version.as_ref().map(|x| x.name.clone()))
    };
    assert_eq!(version(None).as_deref(), Some("SCOPE_2"));
    assert_eq!(version(Some("SCOPE_1")).as_deref(), Some("SCOPE_1"));
    assert_eq!(version(Some("SCOPE_3")), None);
}

#[test]
fn names_a_missing_library_and_who_needs_it() {
    let Some(user) = fixture("libscope_user.so") else {
        return;
    };
    let dir = tempfile::tempdir().expect("temp dir");
    let copy = dir.path().join("libscope_user.so");
    std::fs::copy(user, &copy).expect("copy");
    let mut ns = Namespace::new();
    let result = ns.load(&copy, &mut nothing);
    if cfg!(target_arch = "x86_64") {
        assert!(matches!(
            result,
            Err(Error::NotFound { name, needed_by })
                if name == "libscope_over.so" && needed_by == "libscope_user.so"
        ));
    } else {
        assert!(matches!(result, Err(Error::Misaligned { .. })));
    }
}

#[cfg(target_arch = "x86_64")]
#[test]
fn host_libraries_are_not_loaded_and_answer_what_the_scope_lacks() {
    let Some(path) = fixture("dlopen") else {
        return;
    };
    let mut ns = Namespace::new();
    ns.host_library("libc.so.6").host_library("libdl.so.2");

    // Unanswered, the executable's C library imports fail the load, which leaves
    // nothing behind.
    let failed = ns.load(&path, &mut nothing);
    assert!(matches!(failed, Err(Error::UnresolvedIn { module, .. }) if module == "dlopen"));
    assert!(ns.modules().is_empty());

    // Answered (with a placeholder address; nothing runs), it loads alone.
    let mut asked = Vec::new();
    ns.load(&path, &mut |import| {
        asked.push(import.name.to_owned());
        Some(0x1000)
    })
    .expect("load");
    let names: Vec<_> = ns.modules().iter().map(|m| m.name.as_str()).collect();
    assert_eq!(names, ["dlopen"]);
    for wanted in ["dlopen", "dlsym", "dlclose"] {
        assert!(
            asked.iter().any(|n| n == wanted),
            "{wanted} asked of the host"
        );
    }
}
