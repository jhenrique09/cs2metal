use std::fs;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

use cs2metal_integrity::{Baseline, Difference, Mode, record, verify};

fn write(root: &Path, relative: &str, bytes: &[u8]) {
    let path = root.join(relative);
    fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
    fs::write(path, bytes).expect("write");
}

fn installation() -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("temp dir");
    let root = dir.path();
    write(root, "game/bin/linuxsteamrt64/cs2", b"abc");
    write(
        root,
        "game/bin/linuxsteamrt64/libtier0.so",
        &[7u8; 3 * 1024 * 1024],
    );
    write(root, "game/csgo/pak01_dir.vpk", b"content");
    write(root, "game/empty.txt", b"");
    write(root, "steamapps/appmanifest_730.acf", b"bookkeeping");
    std::os::unix::fs::symlink(
        "libtier0.so",
        root.join("game/bin/linuxsteamrt64/libtier0.so.1"),
    )
    .expect("symlink");
    dir
}

fn baseline_of(root: &Path) -> Baseline {
    record(root, &AtomicU64::new(0)).expect("record")
}

fn differences(root: &Path, baseline: &Baseline, mode: Mode) -> Vec<Difference> {
    verify(root, baseline, mode, &AtomicU64::new(0))
        .expect("verify")
        .differences
}

#[test]
fn records_every_game_file_with_its_sha256() {
    let dir = installation();
    let baseline = baseline_of(dir.path());
    let paths: Vec<_> = baseline.files.iter().map(|f| f.path.as_str()).collect();
    assert_eq!(
        paths,
        [
            "game/bin/linuxsteamrt64/cs2",
            "game/bin/linuxsteamrt64/libtier0.so",
            "game/csgo/pak01_dir.vpk",
            "game/empty.txt",
        ]
    );
    // SHA-256 test vectors: "abc" and the empty string.
    assert_eq!(
        baseline.files[0].sha256,
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
    assert_eq!(
        baseline.files[3].sha256,
        "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
    );
    assert_eq!(baseline.total_bytes(), 3 + 3 * 1024 * 1024 + 7);
    assert_eq!(baseline.excluded, ["steamapps"]);
}

#[test]
fn records_links_without_following_them() {
    let dir = installation();
    let baseline = baseline_of(dir.path());
    assert_eq!(baseline.links.len(), 1);
    assert_eq!(
        baseline.links[0].path,
        "game/bin/linuxsteamrt64/libtier0.so.1"
    );
    assert_eq!(baseline.links[0].target, "libtier0.so");
}

#[test]
fn reports_progress_in_bytes() {
    let dir = installation();
    let progress = AtomicU64::new(0);
    let baseline = record(dir.path(), &progress).expect("record");
    assert_eq!(progress.load(Ordering::Relaxed), baseline.total_bytes());
}

#[test]
fn never_names_the_root_and_survives_a_json_round_trip() {
    let dir = installation();
    let baseline = baseline_of(dir.path());
    let json = baseline.to_json();
    assert!(!json.contains(&*dir.path().to_string_lossy()));
    assert_eq!(Baseline::from_json(&json).expect("parse"), baseline);
}

#[test]
fn rejects_an_unknown_format() {
    let dir = installation();
    let json = baseline_of(dir.path())
        .to_json()
        .replace("\"format\": 1", "\"format\": 99");
    assert!(Baseline::from_json(&json).is_err());
    assert!(Baseline::from_json("not json").is_err());
}

#[test]
fn recording_changes_nothing_on_disk() {
    let dir = installation();
    let file = dir.path().join("game/bin/linuxsteamrt64/cs2");
    let before = fs::metadata(&file).expect("meta");
    baseline_of(dir.path());
    let after = fs::metadata(&file).expect("meta");
    assert_eq!(before.modified().ok(), after.modified().ok());
    assert_eq!(before.len(), after.len());
    assert_eq!(fs::read(&file).expect("read"), b"abc");
}

#[test]
fn an_unchanged_installation_verifies_clean_in_both_modes() {
    let dir = installation();
    let baseline = baseline_of(dir.path());
    let full = verify(dir.path(), &baseline, Mode::Full, &AtomicU64::new(0)).expect("verify");
    assert!(full.is_clean(), "{:?}", full.differences);
    assert_eq!(full.files_hashed, 4);
    let quick = verify(dir.path(), &baseline, Mode::Quick, &AtomicU64::new(0)).expect("verify");
    assert!(quick.is_clean());
    assert_eq!(quick.files_hashed, 0);
}

#[test]
fn detects_same_size_changes_resizes_removals_and_additions() {
    let dir = installation();
    let root = dir.path();
    let baseline = baseline_of(root);
    write(root, "game/bin/linuxsteamrt64/cs2", b"abd");
    write(root, "game/csgo/pak01_dir.vpk", b"longer content");
    fs::remove_file(root.join("game/empty.txt")).expect("remove");
    write(root, "game/new.so", b"x");
    for mode in [Mode::Full, Mode::Quick] {
        assert_eq!(
            differences(root, &baseline, mode),
            [
                Difference::Missing("game/empty.txt".into()),
                Difference::Added("game/new.so".into()),
                Difference::Modified("game/bin/linuxsteamrt64/cs2".into()),
                Difference::Modified("game/csgo/pak01_dir.vpk".into()),
            ],
            "{mode:?}"
        );
    }
}

#[test]
fn detects_link_changes() {
    let dir = installation();
    let root = dir.path();
    let baseline = baseline_of(root);
    let link = root.join("game/bin/linuxsteamrt64/libtier0.so.1");
    fs::remove_file(&link).expect("remove");
    std::os::unix::fs::symlink("cs2", &link).expect("symlink");
    std::os::unix::fs::symlink("cs2", root.join("game/extra")).expect("symlink");
    assert_eq!(
        differences(root, &baseline, Mode::Full),
        [
            Difference::LinkChanged {
                path: "game/bin/linuxsteamrt64/libtier0.so.1".into(),
                expected: "libtier0.so".into(),
                found: "cs2".into(),
            },
            Difference::LinkAdded("game/extra".into()),
        ]
    );
}

#[test]
fn ignores_the_download_tool_bookkeeping() {
    let dir = installation();
    let root = dir.path();
    let baseline = baseline_of(root);
    write(
        root,
        "steamapps/appmanifest_730.acf",
        b"rewritten by steamcmd",
    );
    write(root, "steamapps/downloading/state", b"x");
    assert!(differences(root, &baseline, Mode::Full).is_empty());
}

#[test]
fn rejects_a_root_that_is_not_a_directory() {
    let dir = installation();
    let file = dir.path().join("game/empty.txt");
    assert!(record(&file, &AtomicU64::new(0)).is_err());
    assert!(record(&dir.path().join("missing"), &AtomicU64::new(0)).is_err());
}
