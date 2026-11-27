//! Builds a miniature installation shaped like `game/bin/linuxsteamrt64` from
//! synthetic objects and checks what the inventory attributes to the host.

use std::fs;
use std::path::Path;

use cs2metal_elf::consts::{R_X86_64_COPY, STB_GLOBAL, STT_FUNC, STT_OBJECT};
use cs2metal_elf::testing::ElfBuilder;
use cs2metal_inventory::{Family, markdown, needed_closure, scan};

fn write(root: &Path, relative: &str, bytes: &[u8]) {
    let path = root.join(relative);
    fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
    fs::write(path, bytes).expect("write");
}

fn installation() -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("temp dir");
    let root = dir.path();
    let bin = "game/bin/linuxsteamrt64";

    let exe = ElfBuilder::executable()
        .needed("libtier0.so")
        .needed("libc.so.6")
        .import_versioned("__libc_start_main", "libc.so.6", "GLIBC_2.34")
        .import_kind("stdout", STT_OBJECT, STB_GLOBAL)
        .relocation(R_X86_64_COPY, Some("stdout"))
        .import("Plat_Init")
        .build();
    write(root, &format!("{bin}/cs2"), &exe);

    let tier0 = ElfBuilder::shared_object("libtier0.so")
        .needed("libc.so.6")
        .needed("libpthread.so.0")
        .needed("libSDL3.so.0")
        .import_versioned("pthread_create", "libc.so.6", "GLIBC_2.34")
        .import_versioned("pthread_create", "libc.so.6", "GLIBC_2.34")
        .import("SDL_Init")
        .weak_import("__cxa_finalize")
        .export("Plat_Init", STT_FUNC)
        .export("malloc", STT_FUNC)
        .tls(16, 32, 16)
        .relocation(8, None)
        .build();
    write(root, &format!("{bin}/libtier0.so"), &tier0);

    let client = ElfBuilder::shared_object("libclient.so")
        .needed("libtier0.so")
        .needed("libvulkan.so.1")
        .needed("libstdc++.so.6")
        .import_versioned("_ZNSt8ios_base4InitC1Ev", "libstdc++.so.6", "GLIBCXX_3.4")
        .import_versioned("memcpy", "libc.so.6", "GLIBC_2.14")
        .import_versioned("malloc", "libc.so.6", "GLIBC_2.2.5")
        .import("vkCreateInstance")
        .import("Plat_Init")
        .weak_import("__cxa_finalize")
        .static_tls()
        .build();
    write(root, &format!("{bin}/libclient.so"), &client);

    // A bundled SDL reached through a versioned symlink, as runtimes ship it.
    let sdl = ElfBuilder::shared_object("libSDL3.so.0")
        .export("SDL_Init", STT_FUNC)
        .build();
    write(root, &format!("{bin}/libSDL3.so.0.2.0"), &sdl);
    #[cfg(unix)]
    std::os::unix::fs::symlink(
        root.join(format!("{bin}/libSDL3.so.0.2.0")),
        root.join(format!("{bin}/libSDL3.so.0")),
    )
    .expect("symlink");

    write(root, "game/csgo/pak01_dir.vpk", b"VPK not an ELF");
    write(root, &format!("{bin}/broken.so"), b"\x7fELF\x01\x01\x01");
    write(root, "game/empty", b"");
    dir
}

#[test]
fn finds_every_object_and_skips_the_rest() {
    let dir = installation();
    let inv = scan(dir.path()).expect("scan");
    let paths: Vec<_> = inv.modules.iter().map(|m| m.path.as_str()).collect();
    assert_eq!(
        paths,
        [
            "game/bin/linuxsteamrt64/cs2",
            "game/bin/linuxsteamrt64/libSDL3.so.0.2.0",
            "game/bin/linuxsteamrt64/libclient.so",
            "game/bin/linuxsteamrt64/libtier0.so",
        ]
    );
    assert_eq!(inv.skipped.len(), 1);
    assert_eq!(inv.skipped[0].path, "game/bin/linuxsteamrt64/broken.so");
    assert!(inv.interpreters.contains("/lib64/ld-linux-x86-64.so.2"));
}

#[test]
fn bundled_needs_resolve_through_sonames_and_symlinks() {
    let dir = installation();
    let inv = scan(dir.path()).expect("scan");
    let tier0 = inv
        .module("game/bin/linuxsteamrt64/libtier0.so")
        .expect("tier0");
    let sdl = tier0.needed.iter().find(|n| n.name == "libSDL3.so.0");
    assert_eq!(
        sdl.and_then(|n| n.bundled.as_deref()),
        Some("game/bin/linuxsteamrt64/libSDL3.so.0.2.0")
    );
    assert!(inv.host_library("libSDL3.so.0").is_none());
}

#[test]
fn attributes_versioned_imports_to_host_libraries() {
    let dir = installation();
    let inv = scan(dir.path()).expect("scan");

    let libc = inv.host_library("libc.so.6").expect("libc");
    assert_eq!(libc.family, Family::Glibc);
    let names: Vec<_> = libc.symbols.keys().map(String::as_str).collect();
    assert_eq!(names, ["__libc_start_main", "memcpy", "pthread_create"]);
    let create = &libc.symbols["pthread_create"];
    assert_eq!(create.referenced_by.len(), 1);
    assert!(create.versions.contains("GLIBC_2.34"));

    let cxx = inv.host_library("libstdc++.so.6").expect("libstdc++");
    assert_eq!(cxx.family, Family::GccRuntime);
    assert!(cxx.symbols.contains_key("_ZNSt8ios_base4InitC1Ev"));

    // Needed without any versioned symbol still makes a host requirement.
    let pthread = inv.host_library("libpthread.so.0").expect("libpthread");
    assert!(pthread.symbols.is_empty());
    assert_eq!(
        inv.host_library("libvulkan.so.1").map(|l| l.family),
        Some(Family::Vulkan)
    );
}

#[test]
fn bundled_definitions_interpose_host_symbols() {
    let dir = installation();
    let inv = scan(dir.path()).expect("scan");
    assert_eq!(
        inv.interposed.get("malloc").map(String::as_str),
        Some("game/bin/linuxsteamrt64/libtier0.so")
    );
    let libc = inv.host_library("libc.so.6").expect("libc");
    assert!(!libc.symbols.contains_key("malloc"));
}

#[test]
fn unversioned_imports_resolve_globally_or_stay_unattributed() {
    let dir = installation();
    let inv = scan(dir.path()).expect("scan");
    // Plat_Init is exported by tier0 and SDL_Init by the bundled SDL.
    assert!(!inv.unattributed.contains_key("Plat_Init"));
    assert!(!inv.unattributed.contains_key("SDL_Init"));
    assert!(inv.unattributed.contains_key("vkCreateInstance"));
    let finalize = &inv.unattributed["__cxa_finalize"];
    assert!(finalize.weak_only);
    assert_eq!(finalize.referenced_by.len(), 2);
    // stdout is a COPY-relocated glibc object referenced without a version.
    assert!(!inv.unattributed["stdout"].weak_only);
}

#[test]
fn records_tls_copy_relocations_and_relocation_totals() {
    let dir = installation();
    let inv = scan(dir.path()).expect("scan");
    let exe = inv.module("game/bin/linuxsteamrt64/cs2").expect("exe");
    assert_eq!(exe.copy_relocations, ["stdout"]);
    let client = inv
        .module("game/bin/linuxsteamrt64/libclient.so")
        .expect("client");
    assert!(client.uses_tls && client.static_tls);
    assert_eq!(inv.relocations.get("R_X86_64_COPY"), Some(&1));
    assert_eq!(inv.relocations.get("R_X86_64_RELATIVE"), Some(&1));
}

#[test]
fn needed_closure_follows_load_order() {
    let dir = installation();
    let inv = scan(dir.path()).expect("scan");
    let order = needed_closure(&inv, "game/bin/linuxsteamrt64/cs2");
    assert_eq!(
        order,
        [
            "game/bin/linuxsteamrt64/cs2",
            "game/bin/linuxsteamrt64/libtier0.so",
            "game/bin/linuxsteamrt64/libSDL3.so.0.2.0",
        ]
    );
}

#[test]
fn the_markdown_report_never_names_the_scanned_root() {
    let dir = installation();
    let inv = scan(dir.path()).expect("scan");
    let report = markdown(&inv);
    let root = dir.path().to_string_lossy().into_owned();
    assert!(!report.contains(&root));
    assert!(report.contains("| `libc.so.6` | glibc | 3 |"));
    assert!(report.contains("never commit it"));
}

#[test]
fn rejects_a_root_that_is_not_a_directory() {
    let dir = installation();
    let file = dir.path().join("game/empty");
    assert!(scan(&file).is_err());
    assert!(scan(&dir.path().join("missing")).is_err());
}
