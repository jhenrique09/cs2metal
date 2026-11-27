use cs2metal_codescan::{FsAccess, ModuleScan, markdown, scan_module, scan_path};
use cs2metal_elf::testing::ElfBuilder;

/// Hand-assembled x86-64 code. Offsets are relative to the start of the blob.
fn code() -> (Vec<u8>, Vec<(u64, u64)>) {
    let mut covered: Vec<u8> = Vec::new();
    covered.extend([0x64, 0x48, 0x8b, 0x04, 0x25, 0, 0, 0, 0]); // mov rax, fs:0
    covered.extend([0x64, 0x48, 0x8b, 0x04, 0x25, 0x28, 0, 0, 0]); // mov rax, fs:0x28
    covered.extend([0x64, 0x48, 0x8b, 0x04, 0x25, 0x30, 0, 0, 0]); // mov rax, fs:0x30
    covered.extend([0x64, 0x8b, 0x04, 0x25, 0xfc, 0xff, 0xff, 0xff]); // mov eax, fs:-4
    covered.extend([0x64, 0x8b, 0x00]); // mov eax, fs:[rax]
    covered.extend([0x66, 0x64, 0x8b, 0x00]); // mov ax, fs:[rax]: prefix not first
    covered.extend([0xb8, 0x64, 0, 0, 0]); // mov eax, 0x64: a 0x64 byte, no prefix
    covered.extend([0x65, 0x48, 0x8b, 0x04, 0x25, 0, 0, 0, 0]); // gs, not fs
    covered.push(0xc3); // ret
    let first = covered.len() as u64;

    let mut bytes = covered;
    bytes.extend([0x64, 0x8b, 0x00, 0xc3]); // uncovered: mov eax, fs:[rax]; ret
    let gap = (first, 4u64);
    bytes.extend([0x06, 0xc3]); // covered: invalid in 64-bit mode (push es); ret
    let bad = (gap.0 + gap.1, 2u64);
    (bytes, vec![(0, first), bad])
}

fn scan(builder: ElfBuilder) -> ModuleScan {
    let bytes = builder.build();
    let elf = cs2metal_elf::parse(&bytes).expect("valid object");
    scan_module(&bytes, &elf)
}

fn module() -> ElfBuilder {
    let (bytes, functions) = code();
    ElfBuilder::shared_object("libtls.so")
        .code(&bytes, &functions)
        .tls(8, 64, 16)
        .relocation(16, None)
        .relocation(18, None)
        .relocation(18, None)
}

#[test]
fn classifies_every_fs_access_in_covered_code() {
    let m = scan(module());
    let kinds: Vec<_> = m.fs_accesses.iter().map(|(k, n)| (*k, *n)).collect();
    assert_eq!(
        kinds,
        [
            (FsAccess::ThreadPointer, 1),
            (FsAccess::StackGuard, 1),
            (FsAccess::PointerGuard, 1),
            (FsAccess::ConstantOffset, 1),
            (FsAccess::RegisterRelative, 2),
        ]
    );
    assert_eq!(m.covered_fs(), 6);
    assert_eq!(m.rewritable, 6, "every prefix byte must be located");
}

#[test]
fn measures_coverage_and_what_lies_outside_it() {
    let m = scan(module());
    let (bytes, _) = code();
    assert_eq!(m.functions, 2);
    assert_eq!(m.executable_bytes, bytes.len() as u64);
    assert_eq!(m.covered_bytes, bytes.len() as u64 - 4);
    assert_eq!(m.uncovered_fs, 1);
    assert_eq!(m.decode_failures, 1);
    assert!(!m.rewrite_is_clean());
}

#[test]
fn a_fully_covered_clean_module_is_clean() {
    let code = [0x64, 0x48, 0x8b, 0x04, 0x25, 0x28, 0, 0, 0, 0xc3];
    let m = scan(ElfBuilder::shared_object("libok.so").code(&code, &[(0, code.len() as u64)]));
    assert_eq!(m.coverage(), 1.0);
    assert!(m.rewrite_is_clean());
}

#[test]
fn reports_tls_models_from_relocations_and_code() {
    let m = scan(module().static_tls().relocation(36, None));
    assert_eq!(m.tls.dynamic_module_relocations, 1);
    assert_eq!(m.tls.initial_exec_relocations, 2);
    assert_eq!(m.tls.descriptor_relocations, 1);
    assert_eq!(m.tls.tls_block_bytes, 64);
    assert!(m.tls.static_tls_flag);
    assert_eq!(
        m.tls.summary(&m.fs_accesses),
        ["dynamic", "descriptors", "initial-exec", "local-exec"]
    );
}

#[test]
fn a_module_without_unwind_tables_is_swept_but_never_clean() {
    let code = [0x64, 0x8b, 0x00, 0xc3];
    let m = scan(ElfBuilder::shared_object("libasm.so").code(&code, &[]));
    assert_eq!(m.functions, 0);
    assert_eq!(m.covered_bytes, 0);
    assert_eq!(m.uncovered_fs, 1);
    assert!(!m.rewrite_is_clean());
}

#[test]
fn scans_a_directory_and_renders_a_report_without_the_root() {
    let dir = tempfile::tempdir().expect("temp dir");
    let bin = dir.path().join("game/bin/linuxsteamrt64");
    std::fs::create_dir_all(&bin).expect("mkdir");
    std::fs::write(bin.join("libtls.so"), module().build()).expect("write");
    std::fs::write(bin.join("broken.so"), b"\x7fELF\x01").expect("write");
    std::fs::write(bin.join("notes.txt"), b"text").expect("write");

    let entries = scan_path(dir.path()).expect("scan");
    let paths: Vec<_> = entries.iter().map(|e| e.path.as_str()).collect();
    assert_eq!(
        paths,
        [
            "game/bin/linuxsteamrt64/broken.so",
            "game/bin/linuxsteamrt64/libtls.so"
        ]
    );
    assert!(entries[0].result.is_err());

    let report = markdown(&entries);
    assert!(!report.contains(&*dir.path().to_string_lossy()));
    assert!(report.contains("| Modules scanned | 1 |"));
    assert!(report.contains("| `fs:register` accesses | 2 |"));
}
