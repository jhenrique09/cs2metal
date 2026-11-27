//! Code discovery beyond the unwind tables, on hand-assembled code. Offsets are
//! relative to the start of the code bytes, which load at an address the builder
//! chooses; every reference below is RIP-relative or a relocation, so it holds
//! wherever that is.

use cs2metal_codescan::{FsAccess, ModuleScan, Seed, scan_module};
use cs2metal_elf::consts::{R_X86_64_JUMP_SLOT, R_X86_64_RELATIVE};
use cs2metal_elf::testing::ElfBuilder;

const RET: u8 = 0xc3;
const INT3: u8 = 0xcc;
/// `mov rax, fs:0`
const READ_TP: [u8; 9] = [0x64, 0x48, 0x8b, 0x04, 0x25, 0, 0, 0, 0];

fn rel32(from_end: usize, to: usize) -> [u8; 4] {
    (to as i64 - from_end as i64).to_le_bytes()[..4]
        .try_into()
        .expect("four bytes")
}

/// `call rel32` at `at`, to `to`.
fn call(code: &mut Vec<u8>, to: usize) {
    let at = code.len();
    code.push(0xe8);
    code.extend(rel32(at + 5, to));
}

/// `lea rdi, [rip + disp32]` at the end of `code`, naming `to`.
fn lea_rdi(code: &mut Vec<u8>, to: usize) {
    let at = code.len();
    code.extend([0x48, 0x8d, 0x3d]);
    code.extend(rel32(at + 7, to));
}

/// Pads with int3 to `len`, as compilers pad between functions.
fn pad_to(code: &mut Vec<u8>, len: usize) {
    code.resize(len, INT3);
}

/// A function with no FDE that reads the thread pointer and returns.
fn hidden_function(code: &mut Vec<u8>) {
    code.extend(READ_TP);
    code.push(RET);
}

fn scan(builder: ElfBuilder) -> ModuleScan {
    let bytes = builder.build();
    let elf = cs2metal_elf::parse(&bytes).expect("valid object");
    scan_module(&bytes, &elf)
}

fn assert_found_one_thread_pointer_read(m: &ModuleScan) {
    assert_eq!(
        m.discovery.fs_accesses.get(&FsAccess::ThreadPointer),
        Some(&1)
    );
    assert_eq!(m.discovery.rewritable, 1);
    assert!(m.uncovered_fs.is_empty(), "nothing left unexplained");
    assert!(m.rewrite_is_clean());
}

#[test]
fn follows_a_call_from_unwind_described_code() {
    let mut code = Vec::new();
    call(&mut code, 0x10); // the only function with an FDE
    code.push(RET);
    pad_to(&mut code, 0x10);
    hidden_function(&mut code);
    let m = scan(ElfBuilder::shared_object("libcall.so").code(&code, &[(0, 6)]));
    assert_eq!(m.discovery.seeds.get(&Seed::UnwindCode), Some(&1));
    assert_eq!(m.discovery.reached_bytes, 10);
    assert_found_one_thread_pointer_read(&m);
}

#[test]
fn starts_from_exported_functions_and_code_pointers() {
    let mut code = Vec::new();
    hidden_function(&mut code);
    pad_to(&mut code, 0x10);
    hidden_function(&mut code);
    let m = scan(
        ElfBuilder::shared_object("libseeds.so")
            .code(&code, &[])
            .code_export("exported", 0)
            .code_relocation(R_X86_64_RELATIVE, None, 0x30, Some(0x10)),
    );
    assert_eq!(m.discovery.seeds.get(&Seed::Export), Some(&1));
    assert_eq!(m.discovery.seeds.get(&Seed::CodePointer), Some(&1));
    assert_eq!(
        m.discovery.fs_accesses.get(&FsAccess::ThreadPointer),
        Some(&2)
    );
    assert!(m.uncovered_fs.is_empty());
}

#[test]
fn accepts_a_callback_whose_address_is_taken() {
    let mut code = Vec::new();
    lea_rdi(&mut code, 0x10); // pass the callback to some registration call
    code.push(RET);
    pad_to(&mut code, 0x10);
    hidden_function(&mut code);
    let m = scan(ElfBuilder::shared_object("libcallback.so").code(&code, &[(0, 8)]));
    assert_eq!(m.discovery.seeds.get(&Seed::AddressTaken), Some(&1));
    assert_eq!(m.discovery.rejected, 0);
    assert_found_one_thread_pointer_read(&m);
}

#[test]
fn discards_a_table_whose_address_is_taken_and_counts_its_fs_bytes_as_data() {
    let mut code = Vec::new();
    lea_rdi(&mut code, 0x10); // load a table kept in .text
    code.push(RET);
    pad_to(&mut code, 0x10);
    code.extend([0x64, 0x8b, 0x00]); // decodes as mov eax, fs:[rax] ...
    code.push(0x06); // ... then as push es, invalid in 64-bit mode
    let m = scan(ElfBuilder::shared_object("libtable.so").code(&code, &[(0, 8)]));
    assert_eq!(m.discovery.rejected, 1);
    assert_eq!(m.discovery.rejected_bytes, 3);
    assert!(m.discovery.fs_accesses.is_empty(), "nothing of it is kept");
    assert_eq!(m.data_fs.get(&FsAccess::RegisterRelative), Some(&1));
    assert!(m.uncovered_fs.is_empty());
    assert!(m.rewrite_is_clean());
}

#[test]
fn rejects_a_tentative_component_with_an_fs_string_instruction() {
    let mut code = Vec::new();
    lea_rdi(&mut code, 0x10);
    code.push(RET);
    pad_to(&mut code, 0x10);
    code.extend([0x64, 0xa4, RET]); // movsb with fs: compilers do not emit it
    let m = scan(ElfBuilder::shared_object("libstring.so").code(&code, &[(0, 8)]));
    assert_eq!(m.discovery.rejected, 1);
    assert_eq!(m.data_fs.get(&FsAccess::Unaddressed), Some(&1));
}

#[test]
fn does_not_fall_through_a_call_to_a_noreturn_import() {
    // 0x00: FDE function, calls 0x10.
    // 0x10: no FDE; calls the PLT stub at 0x30, which never returns, so the bytes
    //       after the call are not code by any proof.
    // 0x30: jmp [rip + slot], with the slot at 0x40 bound to __stack_chk_fail.
    let mut code = Vec::new();
    call(&mut code, 0x10);
    code.push(RET);
    pad_to(&mut code, 0x10);
    call(&mut code, 0x30);
    code.extend([0x64, 0x8b, 0x00, RET]); // unreachable: mov eax, fs:[rax]
    pad_to(&mut code, 0x30);
    code.extend([0xff, 0x25]);
    code.extend(rel32(0x36, 0x40));
    pad_to(&mut code, 0x48);
    let m = scan(
        ElfBuilder::shared_object("libnoreturn.so")
            .import("__stack_chk_fail")
            .code(&code, &[(0, 6)])
            .code_relocation(R_X86_64_JUMP_SLOT, Some("__stack_chk_fail"), 0x40, None),
    );
    assert_eq!(m.discovery.noreturn_calls, 1);
    assert!(m.discovery.fs_accesses.is_empty());
    assert_eq!(m.uncovered_fs.get(&FsAccess::RegisterRelative), Some(&1));
    assert!(!m.rewrite_is_clean());
}

#[test]
fn a_call_to_an_ordinary_import_falls_through() {
    let mut code = Vec::new();
    call(&mut code, 0x10);
    code.push(RET);
    pad_to(&mut code, 0x10);
    call(&mut code, 0x30);
    code.extend(READ_TP);
    code.push(RET);
    pad_to(&mut code, 0x30);
    code.extend([0xff, 0x25]);
    code.extend(rel32(0x36, 0x40));
    pad_to(&mut code, 0x48);
    let m = scan(
        ElfBuilder::shared_object("libreturns.so")
            .import("memcpy")
            .code(&code, &[(0, 6)])
            .code_relocation(R_X86_64_JUMP_SLOT, Some("memcpy"), 0x40, None),
    );
    assert_eq!(m.discovery.noreturn_calls, 0);
    assert_found_one_thread_pointer_read(&m);
}

#[test]
fn lists_the_address_of_every_prefix_byte_to_rewrite() {
    let mut code = Vec::new();
    call(&mut code, 0x10);
    code.extend([0x66, 0x64, 0x8b, 0x00]); // mov ax, fs:[rax]: the prefix is second
    code.push(RET);
    pad_to(&mut code, 0x10);
    hidden_function(&mut code);
    let bytes = ElfBuilder::shared_object("libsites.so")
        .code(&code, &[(0, 10)])
        .build();
    let elf = cs2metal_elf::parse(&bytes).expect("valid object");
    let text = elf
        .segments
        .iter()
        .find(|s| s.executable())
        .expect("code")
        .vaddr;
    let m = scan_module(&bytes, &elf);
    let relative: Vec<_> = m.rewrite_sites.iter().map(|a| a - text).collect();
    assert_eq!(relative, [6, 0x10], "unwind-described, then discovered");
    assert_eq!(
        m.rewrite_sites.len() as u64,
        m.rewritable + m.discovery.rewritable
    );
}
