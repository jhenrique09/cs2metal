use std::fs;
use std::process::Command;

use cs2metal_elf::consts::STT_FUNC;
use cs2metal_elf::testing::ElfBuilder;

fn cs2metal() -> Command {
    Command::new(env!("CARGO_BIN_EXE_cs2metal"))
}

#[test]
fn inspect_summarizes_an_object() {
    let dir = tempfile::tempdir().expect("temp dir");
    let lib = dir.path().join("libfoo.so");
    let bytes = ElfBuilder::shared_object("libfoo.so")
        .needed("libc.so.6")
        .import_versioned("malloc", "libc.so.6", "GLIBC_2.2.5")
        .export("foo", STT_FUNC)
        .relocation(8, None)
        .build();
    fs::write(&lib, bytes).expect("write");

    let out = cs2metal().arg("inspect").arg(&lib).output().expect("run");
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("soname:      libfoo.so"), "{text}");
    assert!(text.contains("libc.so.6"), "{text}");
    assert!(text.contains("R_X86_64_RELATIVE"), "{text}");

    let out = cs2metal()
        .args(["inspect", "--json"])
        .arg(&lib)
        .output()
        .expect("run");
    let json: serde_json::Value = serde_json::from_slice(&out.stdout).expect("json");
    assert_eq!(json["soname"], "libfoo.so");
}

#[test]
fn inventory_writes_both_reports() {
    let dir = tempfile::tempdir().expect("temp dir");
    let game = dir.path().join("game/bin/linuxsteamrt64");
    fs::create_dir_all(&game).expect("mkdir");
    let exe = ElfBuilder::executable()
        .needed("libc.so.6")
        .import_versioned("puts", "libc.so.6", "GLIBC_2.2.5")
        .build();
    fs::write(game.join("cs2"), exe).expect("write");

    let reports = dir.path().join("reports");
    let out = cs2metal()
        .arg("inventory")
        .arg(dir.path())
        .arg("--json")
        .arg(reports.join("inventory.json"))
        .arg("--markdown")
        .arg(reports.join("inventory.md"))
        .output()
        .expect("run");
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let md = fs::read_to_string(reports.join("inventory.md")).expect("markdown");
    assert!(md.contains("`libc.so.6`"));
    let json: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(reports.join("inventory.json")).expect("json"))
            .expect("parse");
    assert_eq!(json["modules"][0]["path"], "game/bin/linuxsteamrt64/cs2");
}

#[test]
fn errors_are_reported_without_a_panic() {
    let dir = tempfile::tempdir().expect("temp dir");
    let not_elf = dir.path().join("notes.txt");
    fs::write(&not_elf, "plain text").expect("write");
    let out = cs2metal()
        .arg("inspect")
        .arg(&not_elf)
        .output()
        .expect("run");
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("not an ELF file"));
}
