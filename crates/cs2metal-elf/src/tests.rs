use crate::consts::*;
use crate::testing::{ElfBuilder, HashStyle};
use crate::{Error, ObjectType, SymbolBinding, SymbolKind, parse};

fn libfoo() -> ElfBuilder {
    ElfBuilder::shared_object("libfoo.so")
        .needed("libc.so.6")
        .needed("libm.so.6")
        .runpath("$ORIGIN:$ORIGIN/../lib")
        .import_versioned("malloc", "libc.so.6", "GLIBC_2.2.5")
        .import_versioned("pthread_create", "libc.so.6", "GLIBC_2.34")
        .import_versioned("cosf", "libm.so.6", "GLIBC_2.2.5")
        .weak_import("__gmon_start__")
        .export("foo_init", STT_FUNC)
        .export("foo_table", STT_OBJECT)
}

#[test]
fn reads_identity_and_dynamic_strings() {
    let elf = parse(&libfoo().build()).expect("valid object");
    assert_eq!(elf.object_type, ObjectType::SharedObject);
    assert_eq!(elf.machine, EM_X86_64);
    assert_eq!(elf.machine_name(), "x86-64");
    assert!(elf.dynamic);
    assert!(!elf.is_pie());
    assert_eq!(elf.soname.as_deref(), Some("libfoo.so"));
    assert_eq!(elf.needed, ["libc.so.6", "libm.so.6"]);
    assert_eq!(elf.runpath, ["$ORIGIN", "$ORIGIN/../lib"]);
    assert!(elf.rpath.is_empty());
    assert_eq!(elf.interpreter, None);
}

#[test]
fn imports_carry_the_library_that_defines_their_version() {
    let elf = parse(&libfoo().build()).expect("valid object");
    let imports: Vec<_> = elf
        .imports()
        .map(|s| {
            let v = s.version.as_ref();
            (
                s.name.as_str(),
                v.map(|v| v.name.as_str()),
                v.and_then(|v| v.library.as_deref()),
            )
        })
        .collect();
    assert_eq!(
        imports,
        [
            ("malloc", Some("GLIBC_2.2.5"), Some("libc.so.6")),
            ("pthread_create", Some("GLIBC_2.34"), Some("libc.so.6")),
            ("cosf", Some("GLIBC_2.2.5"), Some("libm.so.6")),
            ("__gmon_start__", None, None),
        ]
    );
    let weak = elf.imports().find(|s| s.name == "__gmon_start__");
    assert_eq!(weak.map(|s| s.binding), Some(SymbolBinding::Weak));

    let mut needs: Vec<_> = elf
        .version_needs
        .iter()
        .map(|n| (n.library.as_str(), n.versions.clone()))
        .collect();
    needs.sort();
    assert_eq!(
        needs,
        [
            (
                "libc.so.6",
                vec!["GLIBC_2.2.5".to_owned(), "GLIBC_2.34".to_owned()]
            ),
            ("libm.so.6", vec!["GLIBC_2.2.5".to_owned()]),
        ]
    );
}

#[test]
fn exports_carry_defined_versions_and_hidden_flags() {
    let bytes = ElfBuilder::shared_object("libbar.so.1")
        .export_versioned("bar_v1", "BAR_1.0", true)
        .export_versioned("bar_v2", "BAR_2.0", false)
        .export("bar_plain", STT_FUNC)
        .build();
    let elf = parse(&bytes).expect("valid object");
    assert_eq!(elf.version_definitions, ["BAR_1.0", "BAR_2.0"]);
    let exports: Vec<_> = elf
        .exports()
        .map(|s| {
            (
                s.name.as_str(),
                s.version.as_ref().map(|v| (v.name.as_str(), v.hidden)),
            )
        })
        .collect();
    assert_eq!(
        exports,
        [
            ("bar_v1", Some(("BAR_1.0", true))),
            ("bar_v2", Some(("BAR_2.0", false))),
            ("bar_plain", None),
        ]
    );
    let kinds: Vec<_> = elf.exports().map(|s| s.kind).collect();
    assert_eq!(kinds, [SymbolKind::Func; 3]);
}

#[test]
fn every_way_of_sizing_the_symbol_table_agrees() {
    let expected = parse(&libfoo().build()).expect("gnu hash").symbols;
    assert_eq!(expected.len(), 6);
    for style in [HashStyle::Sysv, HashStyle::SectionsOnly] {
        let elf = parse(&libfoo().hash_style(style).build()).expect("valid object");
        assert_eq!(elf.symbols, expected, "{style:?}");
    }
}

#[test]
fn gnu_hash_with_only_undefined_symbols() {
    let bytes = ElfBuilder::shared_object("libonly.so")
        .import("a")
        .import("b")
        .build();
    let elf = parse(&bytes).expect("valid object");
    assert_eq!(elf.imports().count(), 2);
    assert_eq!(elf.exports().count(), 0);
}

#[test]
fn an_unsizable_symbol_table_is_an_error() {
    let bytes = libfoo().hash_style(HashStyle::None).build();
    assert_eq!(parse(&bytes), Err(Error::UnknownSymbolCount));
}

#[test]
fn counts_relocations_and_names_copy_symbols() {
    let bytes = ElfBuilder::executable()
        .needed("libc.so.6")
        .import_kind("stdout", STT_OBJECT, STB_GLOBAL)
        .import_kind("environ", STT_OBJECT, STB_GLOBAL)
        .import("puts")
        .relocation(8, None)
        .relocation(8, None)
        .relocation(R_X86_64_COPY, Some("stdout"))
        .relocation(R_X86_64_COPY, Some("environ"))
        .relocation(18, Some("stdout"))
        .plt_relocation(7, "puts")
        .relr(3)
        .build();
    let elf = parse(&bytes).expect("valid object");
    let named: Vec<_> = elf
        .relocations
        .counts
        .iter()
        .map(|(&k, &n)| (x86_64_relocation_name(k).unwrap_or("?"), n))
        .collect();
    assert_eq!(
        named,
        [
            ("R_X86_64_COPY", 2),
            ("R_X86_64_JUMP_SLOT", 1),
            ("R_X86_64_RELATIVE", 2),
            ("R_X86_64_TPOFF64", 1),
        ]
    );
    assert_eq!(elf.relocations.total(), 6);
    assert_eq!(elf.relocations.relr_words, 3);
    assert_eq!(elf.relocations.copy_symbols, ["stdout", "environ"]);
}

#[test]
fn executables_translate_addresses_through_their_load_segment() {
    let elf = parse(&ElfBuilder::executable().import("puts").build()).expect("valid object");
    assert_eq!(elf.object_type, ObjectType::Executable);
    assert_eq!(
        elf.interpreter.as_deref(),
        Some("/lib64/ld-linux-x86-64.so.2")
    );
    assert_eq!(elf.entry, 0x40_1000);
    let (lo, hi) = elf.load_span().expect("one load segment");
    assert_eq!(lo, 0x40_0000);
    assert!(hi > lo && hi % 0x1000 == 0);
}

#[test]
fn pie_is_a_shared_object_with_an_interpreter() {
    let elf = parse(&ElfBuilder::pie().import("puts").build()).expect("valid object");
    assert!(elf.is_pie());
}

#[test]
fn reads_tls_template_flags_and_initializers() {
    let bytes = libfoo()
        .tls(24, 64, 16)
        .static_tls()
        .bind_now()
        .init_array(3)
        .load_bias(0x7000_0000)
        .build();
    let elf = parse(&bytes).expect("valid object");
    let tls = elf.tls.expect("tls segment");
    assert_eq!((tls.file_size, tls.memory_size, tls.align), (24, 64, 16));
    assert!(elf.flags.static_tls);
    assert!(elf.flags.bind_now);
    assert!(!elf.flags.text_relocations);
    assert_eq!(elf.initializers.init_array_entries, 3);
    let tls_segment = elf.segments.iter().find(|s| s.kind == PT_TLS);
    assert_eq!(tls_segment.map(|s| s.name()), Some("TLS"));
    let load = elf.segments.iter().find(|s| s.kind == PT_LOAD);
    assert_eq!(load.map(|s| s.permissions()), Some("r-x".to_owned()));
}

#[test]
fn reports_foreign_machines_without_failing() {
    let elf = parse(&libfoo().machine(EM_AARCH64).build()).expect("valid object");
    assert_eq!(elf.machine_name(), "AArch64");
}

#[test]
fn rejects_what_is_not_elf64_little_endian() {
    assert_eq!(parse(b""), Err(Error::NotElf));
    assert_eq!(parse(b"MZ\x90\x00"), Err(Error::NotElf));
    let mut bytes = libfoo().build();
    bytes[4] = 1;
    assert_eq!(parse(&bytes), Err(Error::UnsupportedClass(1)));
    bytes[4] = ELFCLASS64;
    bytes[5] = 2;
    assert_eq!(parse(&bytes), Err(Error::UnsupportedEncoding(2)));
    bytes[5] = ELFDATA2LSB;
    bytes[6] = 0;
    assert_eq!(parse(&bytes), Err(Error::UnsupportedVersion(0)));
}

#[test]
fn every_truncation_fails_cleanly() {
    let bytes = libfoo().tls(8, 8, 8).relocation(8, None).build();
    for len in 0..bytes.len() {
        // The assertion is that this returns at all: no panic, no hang.
        let _ = parse(&bytes[..len]);
    }
    assert!(parse(&bytes).is_ok());
}

#[test]
fn every_single_byte_corruption_fails_cleanly() {
    let bytes = libfoo()
        .relocation(8, None)
        .plt_relocation(7, "malloc")
        .build();
    for at in 0..bytes.len() {
        for value in [0x00, 0xff, 0x7f] {
            let mut corrupted = bytes.clone();
            corrupted[at] = value;
            let _ = parse(&corrupted);
        }
    }
}

#[test]
fn reads_function_ranges_from_the_unwind_tables() {
    // Three functions; the bytes between 0x20 and 0x30 have no FDE.
    let code = [0x90u8; 0x40];
    let bytes = libfoo()
        .code(&code, &[(0x30, 0x10), (0x00, 0x08), (0x08, 0x18)])
        .load_bias(0x10_0000)
        .build();
    let elf = parse(&bytes).expect("valid object");
    let text = elf
        .segments
        .iter()
        .find(|s| s.kind == PT_LOAD && s.executable())
        .expect("code segment");
    let ranges = crate::unwind_ranges(&bytes, &elf).expect("unwind tables");
    let relative: Vec<_> = ranges
        .iter()
        .map(|r| (r.start - text.vaddr, r.end - text.vaddr))
        .collect();
    assert_eq!(relative, [(0x00, 0x08), (0x08, 0x20), (0x30, 0x40)]);
    let gnu = elf.segments.iter().find(|s| s.kind == PT_GNU_EH_FRAME);
    assert_eq!(gnu.map(|s| s.name()), Some("GNU_EH_FRAME"));
}

#[test]
fn reads_cxx_style_tables_with_an_indirect_personality() {
    let bytes = libfoo()
        .code(&[0xc3u8; 0x20], &[(0x10, 0x10), (0x00, 0x10)])
        .cxx_unwind_tables()
        .build();
    let elf = parse(&bytes).expect("valid object");
    let ranges = crate::unwind_ranges(&bytes, &elf).expect("unwind tables");
    assert_eq!(ranges.len(), 2);
    assert_eq!(ranges[0].end, ranges[1].start);
}

#[test]
fn an_object_without_unwind_tables_has_no_ranges() {
    let bytes = libfoo().build();
    let elf = parse(&bytes).expect("valid object");
    assert_eq!(crate::unwind_ranges(&bytes, &elf), Ok(Vec::new()));
}

#[test]
fn corrupted_unwind_tables_fail_cleanly() {
    let bytes = libfoo().code(&[0xc3u8; 32], &[(0, 16), (16, 16)]).build();
    let elf = parse(&bytes).expect("valid object");
    for at in 0..bytes.len() {
        for value in [0x00, 0xff, 0x7f] {
            let mut corrupted = bytes.clone();
            corrupted[at] = value;
            if let Ok(e) = parse(&corrupted) {
                let _ = crate::unwind_ranges(&corrupted, &e);
            }
            let _ = crate::unwind_ranges(&corrupted, &elf);
        }
    }
}

#[test]
fn reads_executable_sections_from_the_section_table() {
    let bytes = libfoo()
        .code(&[0xc3u8; 0x40], &[])
        .code_sections(&[(0x20, 0x10), (0x00, 0x08)])
        .load_bias(0x10_0000)
        .build();
    let elf = parse(&bytes).expect("valid object");
    let text = elf
        .segments
        .iter()
        .find(|s| s.kind == PT_LOAD && s.executable())
        .expect("code segment");
    let sections = crate::executable_sections(&bytes)
        .expect("section table")
        .expect("present");
    let relative: Vec<_> = sections
        .iter()
        .map(|s| (s.start - text.vaddr, s.end - text.vaddr))
        .collect();
    assert_eq!(relative, [(0x00, 0x08), (0x20, 0x30)]);
}

#[test]
fn an_object_without_a_section_table_has_no_executable_sections() {
    assert_eq!(crate::executable_sections(&libfoo().build()), Ok(None));
    // A table with only a dynamic symbol section names no code.
    let bytes = libfoo().hash_style(HashStyle::SectionsOnly).build();
    assert_eq!(crate::executable_sections(&bytes), Ok(Some(Vec::new())));
}

#[test]
fn corrupted_section_tables_fail_cleanly() {
    let bytes = libfoo()
        .code(&[0xc3u8; 16], &[])
        .code_sections(&[(0, 16)])
        .build();
    for at in 0..bytes.len() {
        for value in [0x00, 0xff, 0x7f] {
            let mut corrupted = bytes.clone();
            corrupted[at] = value;
            let _ = crate::executable_sections(&corrupted);
        }
    }
    let mut wrong_size = bytes.clone();
    wrong_size[58] = 40;
    assert!(matches!(
        crate::executable_sections(&wrong_size),
        Err(crate::Error::BadEntrySize { .. })
    ));
}

#[test]
fn keeps_every_relocation_entry_with_its_symbol_and_addend() {
    let bytes = libfoo()
        .import("abort")
        .code(&[0xc3u8; 0x20], &[])
        .code_export("entry", 0x10)
        .code_relocation(R_X86_64_JUMP_SLOT, Some("abort"), 0x18, None)
        .code_relocation(R_X86_64_RELATIVE, None, 0x08, Some(0x10))
        .build();
    let elf = parse(&bytes).expect("valid object");
    let text = elf
        .segments
        .iter()
        .find(|s| s.kind == PT_LOAD && s.executable())
        .expect("code segment")
        .vaddr;
    let entries = &elf.relocations.entries;
    assert_eq!(entries.len(), 2);
    assert_eq!(entries[0].offset, text + 0x18);
    let bound = elf.relocation_symbol(&entries[0]).map(|s| s.name.as_str());
    assert_eq!(bound, Some("abort"));
    assert_eq!(entries[1].kind, R_X86_64_RELATIVE);
    assert_eq!(entries[1].addend, (text + 0x10) as i64);
    assert_eq!(elf.relocation_symbol(&entries[1]), None);
    let export = elf.exports().find(|s| s.name == "entry").expect("export");
    assert_eq!(export.value, text + 0x10);
}
