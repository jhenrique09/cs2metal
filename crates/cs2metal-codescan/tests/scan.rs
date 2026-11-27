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
    assert!(!m.code_from_sections);
    assert_eq!(m.code_bytes, bytes.len() as u64);
    assert_eq!(m.covered_bytes, bytes.len() as u64 - 4);
    assert_eq!(m.uncovered_total(), 1);
    assert_eq!(m.uncovered_fs.get(&FsAccess::RegisterRelative), Some(&1));
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
    assert_eq!(m.uncovered_total(), 1);
    assert!(!m.rewrite_is_clean());
}

/// Code followed by read-only data in the same executable segment, as some linkers
/// lay objects out. The data is ASCII text: "d\n" followed by NUL is 0x64 0x0a 0x00,
/// which decodes as `or al, fs:[rax]`.
fn code_then_strings() -> Vec<u8> {
    let mut bytes = vec![0x64, 0x48, 0x8b, 0x04, 0x25, 0x28, 0, 0, 0, 0xc3]; // fs:0x28; ret
    bytes.extend_from_slice(b"%d\n\0%d\n\0");
    bytes
}

#[test]
fn data_mapped_with_code_is_not_swept_when_sections_bound_the_code() {
    let bytes = code_then_strings();
    let without = scan(ElfBuilder::shared_object("liblayout.so").code(&bytes, &[(0, 10)]));
    assert!(
        without.uncovered_total() > 0,
        "strings decode as fs accesses"
    );

    let with = scan(
        ElfBuilder::shared_object("liblayout.so")
            .code(&bytes, &[(0, 10)])
            .code_sections(&[(0, 10)]),
    );
    assert!(with.code_from_sections);
    assert_eq!(with.code_bytes, 10);
    assert_eq!(with.coverage(), 1.0);
    assert_eq!(with.uncovered_total(), 0);
    assert!(with.rewrite_is_clean());
}

#[test]
fn uncovered_code_inside_a_section_is_still_counted_by_kind() {
    let mut bytes = code_then_strings();
    bytes.truncate(10);
    bytes.extend([0x64, 0x48, 0x8b, 0x04, 0x25, 0, 0, 0, 0, 0xc3]); // mov rax, fs:0; ret
    let m = scan(
        ElfBuilder::shared_object("libnofde.so")
            .code(&bytes, &[(0, 10)])
            .code_sections(&[(0, 20)]),
    );
    assert_eq!(m.uncovered_fs.get(&FsAccess::ThreadPointer), Some(&1));
    assert_eq!(m.uncovered_total(), 1);
    assert!(!m.rewrite_is_clean());
    let report = markdown(&[cs2metal_codescan::Entry {
        path: "libnofde.so".to_owned(),
        result: Ok(m),
    }]);
    assert!(report.contains("| `fs` decodes unexplained | fs:0 1 |"));
    assert!(report.contains("| sections |"));
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
    assert!(report.contains(
        "| `fs` in unwind-described code | fs:0 1, fs:0x28 1, fs:0x30 1, fs:constant 1, fs:register 2 |"
    ));
}

#[test]
fn string_instructions_are_not_thread_pointer_reads() {
    // movsb byte [rdi], fs:[rsi]; outsb dx, fs:[rsi]; ret, with no FDE.
    let code = [0x64, 0xa4, 0x64, 0x6e, 0xc3];
    let m = scan(
        ElfBuilder::shared_object("libdata.so")
            .code(&code, &[])
            .code_sections(&[(0, code.len() as u64)]),
    );
    assert_eq!(m.uncovered_fs.get(&FsAccess::Unaddressed), Some(&2));
    assert_eq!(m.uncovered_fs.get(&FsAccess::ThreadPointer), None);
}

#[test]
fn a_thread_pointer_load_is_local_exec_only_in_an_executable_with_tls() {
    // mov rax, fs:0; lea rax, [rax - 8]; ret: how gcc reaches a local-exec variable.
    let code = [
        0x64, 0x48, 0x8b, 0x04, 0x25, 0, 0, 0, 0, 0x48, 0x8d, 0x40, 0xf8, 0xc3,
    ];
    let fde = [(0, code.len() as u64)];
    let exe = scan(ElfBuilder::pie().tls(0, 16, 8).code(&code, &fde));
    assert_eq!(exe.tls_summary(), ["local-exec"]);
    let lib = scan(
        ElfBuilder::shared_object("libtp.so")
            .tls(0, 16, 8)
            .code(&code, &fde),
    );
    assert!(
        lib.tls_summary().is_empty(),
        "a library may not use local-exec"
    );
    let no_tls = scan(ElfBuilder::pie().code(&code, &fde));
    assert!(no_tls.tls_summary().is_empty());
}
