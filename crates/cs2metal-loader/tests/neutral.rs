//! The probe that shows whether code is neutral: it lets code that leaves GS alone
//! through, and reports code that writes through it.
#![cfg(target_arch = "x86_64")]

use cs2metal_loader::{Error, probe_neutral};

extern "C" fn doubles(value: u64) -> u64 {
    value * 2
}

/// Writes a word at `gs:[offset]`, as libSystem does to a thread's data.
extern "C" fn writes_thread_data(offset: u64) -> u64 {
    // SAFETY: under the probe GS points at a writable block one page long, and
    // the offsets used here lie inside it.
    unsafe {
        core::arch::asm!(
            "mov qword ptr gs:[{o}], 1",
            o = in(reg) offset,
            options(nostack, preserves_flags),
        );
    }
    0
}

/// Reads its own thread pointer as the guest does, at `gs:0x10`.
extern "C" fn reads_the_control_block(_: u64) -> u64 {
    let word: u64;
    // SAFETY: as above; a read of the block's third word.
    unsafe {
        core::arch::asm!(
            "mov {w}, qword ptr gs:[0x10]",
            w = out(reg) word,
            options(nostack, readonly, preserves_flags),
        );
    }
    word
}

#[test]
fn code_that_leaves_gs_alone_is_neutral() {
    assert_eq!(probe_neutral(doubles, 21).expect("neutral"), 42);
}

#[test]
fn a_write_through_gs_is_reported_with_its_offset() {
    // 0x8 is where macOS keeps a thread's errno pointer.
    for offset in [0x8u64, 0x100, 0xff8] {
        match probe_neutral(writes_thread_data, offset) {
            Err(Error::NotNeutral { offset: found }) => assert_eq!(found, offset),
            other => panic!("a write at {offset:#x} gave {other:?}"),
        }
    }
}

#[test]
fn the_block_holds_poison_that_is_no_address() {
    let word = probe_neutral(reads_the_control_block, 0).expect("a read changes nothing");
    assert_eq!(
        word >> 48,
        0xdead,
        "{word:#x} is not canonical: following it faults"
    );
}

#[test]
fn the_host_domain_is_back_after_a_probe() {
    probe_neutral(doubles, 1).expect("neutral");
    // Thread-local storage and errno live behind the host's GS base.
    thread_local!(static CELL: std::cell::Cell<u32> = const { std::cell::Cell::new(7) });
    assert_eq!(CELL.with(std::cell::Cell::get), 7);
    assert_eq!(std::thread::current().id(), std::thread::current().id());
}
