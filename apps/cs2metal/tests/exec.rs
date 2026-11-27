//! Runs the Linux fixtures under `cs2metal exec`, with the rebuilt glibc of
//! tools/guest-glibc.sh and the C++ runtime of the Steam Linux Runtime, and
//! compares each with the output it printed natively on Linux
//! (fixtures/expected). Only an x86-64 build runs guest code.
#![cfg(target_arch = "x86_64")]

use std::path::{Path, PathBuf};
use std::process::Command;

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// The runtime's C++ libraries: CS2METAL_RUNTIME_LIBS, or the owner's kit.
fn runtime_libs() -> Option<PathBuf> {
    let dir = std::env::var_os("CS2METAL_RUNTIME_LIBS")
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME")
                .map(|h| PathBuf::from(h).join("cs2metal-kit/sniper/files/lib/x86_64-linux-gnu"))
        })?;
    dir.join("libstdc++.so.6").exists().then_some(dir)
}

fn run(name: &str, args: &[&str], needs_cxx: bool) {
    let fixtures = root().join("target/fixtures");
    let glibc = root().join("target/guest-glibc");
    if !fixtures.join(name).exists() || !glibc.join("ld-linux-x86-64.so.2").exists() {
        eprintln!("skipped: {name}; run tools/fixtures.sh and tools/guest-glibc.sh");
        return;
    }
    let mut command = Command::new(env!("CARGO_BIN_EXE_cs2metal"));
    command
        .current_dir(&fixtures)
        .arg("exec")
        .arg("--glibc")
        .arg(&glibc);
    if needs_cxx {
        let Some(libs) = runtime_libs() else {
            eprintln!("skipped: {name}; the Steam Linux Runtime's libstdc++ was not found");
            return;
        };
        command.arg("--library-path").arg(libs);
    }
    let out = command
        .args([
            "--env",
            "CS2METAL_FIXTURE=set",
            "--env",
            "PATH=/usr/bin:/bin",
        ])
        .arg(name)
        .args(args)
        .output()
        .expect("run cs2metal exec");
    let status = out.status.code().expect("an exit status");
    let actual = format!("{}exit: {status}", String::from_utf8_lossy(&out.stdout));
    let expected =
        std::fs::read_to_string(root().join("fixtures/expected").join(format!("{name}.txt")))
            .expect("expected output");
    assert_eq!(
        actual,
        expected.trim_end(),
        "{name} under cs2metal; stderr:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn hello() {
    run("hello", &["one", "two words"], false);
}

#[test]
fn stack_guard() {
    run("stack_guard", &[], false);
}

#[test]
fn ifunc() {
    run("ifunc", &[], false);
}

#[test]
fn scope() {
    run("scope", &[], false);
}

#[test]
fn dlopen() {
    run("dlopen", &[], false);
}

#[test]
fn cxx() {
    run("cxx", &[], true);
}

#[test]
fn tls_from_four_threads() {
    run("tls", &[], false);
}

#[test]
fn tlsprobe_from_four_threads() {
    run("tlsprobe", &[], false);
}
