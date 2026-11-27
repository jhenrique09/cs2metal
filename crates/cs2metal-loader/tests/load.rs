use std::path::{Path, PathBuf};

use cs2metal_elf::consts::{EM_AARCH64, R_X86_64_GLOB_DAT};
use cs2metal_elf::testing::ElfBuilder;
use cs2metal_loader::{Error, Image, Import};

fn write(dir: &Path, name: &str, bytes: &[u8]) -> PathBuf {
    let path = dir.join(name);
    std::fs::write(&path, bytes).expect("write");
    path
}

fn nothing(_: Import<'_>) -> Option<u64> {
    None
}

#[test]
fn refuses_what_it_cannot_place() {
    let dir = tempfile::tempdir().expect("temp dir");
    let text = write(dir.path(), "notes.txt", b"not an object");
    assert!(matches!(
        Image::load(&text, &mut nothing),
        Err(Error::Elf(_))
    ));

    let arm = ElfBuilder::shared_object("libarm.so")
        .machine(EM_AARCH64)
        .build();
    let arm = write(dir.path(), "libarm.so", &arm);
    assert!(matches!(
        Image::load(&arm, &mut nothing),
        Err(Error::WrongMachine(EM_AARCH64))
    ));

    let fixed = write(dir.path(), "fixed", &ElfBuilder::executable().build());
    assert!(matches!(
        Image::load(&fixed, &mut nothing),
        Err(Error::NotRelocatable)
    ));
}

#[test]
fn a_strong_import_nothing_provides_is_an_error_and_names_it() {
    let dir = tempfile::tempdir().expect("temp dir");
    let lib = ElfBuilder::shared_object("libneeds.so")
        .import("puts")
        .relocation(R_X86_64_GLOB_DAT, Some("puts"))
        .build();
    let lib = write(dir.path(), "libneeds.so", &lib);
    let mut asked = Vec::new();
    let result = Image::load(&lib, &mut |import| {
        asked.push((import.name.to_owned(), import.weak));
        None
    });
    assert!(matches!(result, Err(Error::Unresolved(name)) if name == "puts"));
    assert_eq!(asked, [("puts".to_owned(), false)]);
}

/// A fixture built by tools/fixtures.sh, or None (with a note) when it is absent.
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

/// Linux x86-64 objects align segments to 4 KiB. A Rosetta 2 process has 4 KiB
/// pages, a native arm64 process 16 KiB, where the same file cannot be mapped.
#[cfg(not(target_arch = "x86_64"))]
#[test]
fn a_native_arm64_process_cannot_map_linux_segments() {
    let Some(path) = fixture("libdlopen_plugin.so") else {
        return;
    };
    assert!(matches!(
        Image::load(&path, &mut nothing),
        Err(Error::Misaligned { page: 16384, .. })
    ));
}

#[cfg(target_arch = "x86_64")]
#[test]
fn maps_the_plugin_fixture_and_reports_its_weak_imports() {
    let Some(path) = fixture("libdlopen_plugin.so") else {
        return;
    };
    let mut asked = Vec::new();
    let image = Image::load(&path, &mut |import| {
        asked.push((import.name.to_owned(), import.weak));
        None
    })
    .expect("load");
    asked.sort();
    let names: Vec<_> = asked.iter().map(|(n, w)| (n.as_str(), *w)).collect();
    assert_eq!(
        names,
        [
            ("_ITM_deregisterTMCloneTable", true),
            ("_ITM_registerTMCloneTable", true),
            ("__cxa_finalize", true),
            ("__gmon_start__", true),
        ]
    );
    assert!(image.symbol("plugin_answer").is_some());
    assert!(image.symbol("plugin_absent").is_none());
}

/// Runs guest code: the plugin's constructor, two of its functions, its finalizers.
#[cfg(target_arch = "x86_64")]
#[test]
fn runs_the_plugin_fixture_under_rosetta() {
    let Some(path) = fixture("libdlopen_plugin.so") else {
        return;
    };
    let mut image = Image::load(&path, &mut nothing).expect("load");
    // SAFETY: the plugin needs no C library and no TLS; its only imports are weak
    // hooks that may be null.
    unsafe { image.run_initializers() }.expect("initializers");

    let answer = image.symbol("plugin_answer").expect("plugin_answer");
    let name = image.symbol("plugin_name").expect("plugin_name");
    // SAFETY: both are C functions of the loaded plugin with these signatures,
    // declared in fixtures/src/dlopen_plugin.c.
    let (answer, name) = unsafe {
        (
            std::mem::transmute::<usize, extern "C" fn(i32) -> i32>(answer as usize),
            std::mem::transmute::<usize, extern "C" fn() -> *const std::ffi::c_char>(name as usize),
        )
    };
    // The constructor ran once, so the plugin adds 1: 20 * 2 + 1.
    assert_eq!(answer(20), 41);
    // SAFETY: plugin_name returns a pointer to a NUL-terminated string in the
    // plugin's read-only data, which stays mapped while the image lives.
    let text = unsafe { std::ffi::CStr::from_ptr(name()) };
    assert_eq!(text.to_str(), Ok("dlopen plugin"));
    // SAFETY: as for the initializers.
    unsafe { image.run_finalizers() }.expect("finalizers");
}
