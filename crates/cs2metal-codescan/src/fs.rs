//! Classification of `fs`-prefixed instructions.

use iced_x86::{Instruction, OpKind, Register};

use crate::FsAccess;

const FS_PREFIX: u8 = 0x64;

/// Legacy prefixes that may precede an opcode, REX, VEX, or EVEX byte.
fn is_legacy_prefix(b: u8) -> bool {
    matches!(
        b,
        0x26 | 0x2e | 0x36 | 0x3e | 0x64 | 0x65 | 0x66 | 0x67 | 0xf0 | 0xf2 | 0xf3
    )
}

/// The kind of an `fs` access, or `None` if the instruction does not use `fs`.
pub(crate) fn access(instruction: &Instruction) -> Option<FsAccess> {
    (instruction.segment_prefix() == Register::FS).then(|| classify(instruction))
}

fn classify(instruction: &Instruction) -> FsAccess {
    // String instructions address through rsi/rdi without a ModRM memory operand;
    // iced then reports no base and a zero displacement, which is not fs:0.
    let addressed = (0..instruction.op_count()).any(|i| instruction.op_kind(i) == OpKind::Memory);
    if !addressed {
        return FsAccess::Unaddressed;
    }
    if instruction.memory_base() != Register::None || instruction.memory_index() != Register::None {
        return FsAccess::RegisterRelative;
    }
    match instruction.memory_displacement64() {
        0 => FsAccess::ThreadPointer,
        0x28 => FsAccess::StackGuard,
        0x30 => FsAccess::PointerGuard,
        _ => FsAccess::ConstantOffset,
    }
}

/// Whether the instruction's bytes carry exactly one `0x64` among their leading
/// legacy prefixes, so that changing it to `0x65` changes nothing else.
pub(crate) fn prefix_located(bytes: &[u8]) -> bool {
    let prefixes = bytes.iter().take_while(|&&b| is_legacy_prefix(b));
    prefixes.filter(|&&b| b == FS_PREFIX).count() == 1
}
