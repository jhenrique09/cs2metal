# Code discovery beyond the unwind tables, 27 November 2026

## Status

Measured on 27 November 2026 on build 26879581 of the CS2 Linux build, for roadmap
step B6. Follows the [inventory and first scan](LINUX_BUILD_INVENTORY_2026-11-27.md)
of the same build, which found that `.eh_frame` describes only 48.8% of the code and
that `libtier0.so` has 10 real thread-pointer reads outside it. Not edited after the
fact.

## Question

Can a rule that proves bytes are code, independent of the unwind tables, reach the
10 thread-pointer reads in `libtier0.so` without reaching any of the 523 `fs`
decodes that a linear sweep finds in data embedded in `.text`?

## Method

`cs2metal tls scan` gained a discovery pass:

1. The FDE ranges are decoded linearly, as before.
2. **Trusted paths** start from places where the module itself declares code: direct
   call and jump targets in FDE code, exported functions, the entry point,
   `DT_INIT` and `DT_FINI`, and the targets of `R_X86_64_RELATIVE`,
   `R_X86_64_IRELATIVE`, and function-symbol `R_X86_64_64` relocations (vtables,
   initializer arrays, function pointer tables). Recursive descent follows direct
   calls and jumps, and stops at returns, indirect jumps, traps, and calls to
   functions known not to return. Those are resolved through the PLT and GOT, for
   example `__stack_chk_fail`, `abort`, `__cxa_throw`, and `_Unwind_Resume`.
3. **Tentative paths** start at the target of each RIP-relative `lea` in accepted
   code. Such a target is a callback as often as a table. Each one is followed with
   everything it reaches, and the whole component is discarded on the first overlap
   with decoded code, invalid encoding, or `fs` string instruction.
4. The bytes no accepted path reaches are swept linearly to count `fs` decodes.
   Those at or after the start of a discarded component, in the same unexplained
   run, are counted as **data the code refers to**. The rest are **unexplained**.

The method was tested on hand-assembled objects (16 tests), then run on the 61
modules of the build. The real thread-pointer reads were checked against
`objdump -d`.

## Results

| Measure | Value |
| --- | --- |
| Code bytes | 254,118,761 |
| Described by FDEs | 48.8% |
| Reached by discovery beyond them | 43.0% |
| Proven code in total | 91.9% |
| Conflicts and invalid encodings on trusted paths | 0 and 0 |
| `lea` components accepted / discarded | 36,673 / 268 (6,234 bytes) |
| `fs` accesses in discovered code | 10, all `fs:0`, all in `libtier0.so`, all prefix-located |
| `fs` decodes in data the code refers to | 514 |
| `fs` decodes unexplained | 9, all string instructions (`fs:unaddressed`) |
| Modules whose rewrite is clean | 61 of 61 |

Seeds that led into unexplored code:

| Seed | Count |
| --- | --- |
| Direct branch from FDE code | 1,051,244 |
| Code pointer in a relocation | 605,774 |
| `lea` target accepted | 36,673 |
| Exported function | 6,996 |
| `DT_INIT` or `DT_FINI` | 122 |
| Entry point | 7 |

`libtier0.so` goes from 16.2% to 87.3% proven code, and all 10 thread-pointer reads
are reached, the same 10 that `objdump` shows. Without the `lea` rule, trusted
paths alone reach 9 of them. The tenth is in code reached only through a `lea` that
loads its address, the pattern of a callback. (INFERRED from which rule reached it)

None of the 523 data decodes is reached. 514 lie in blocks that accepted code loads
by `lea` and that fail to decode as code. The other 9 are `fs`-prefixed string
instructions (`movs`, `cmps`, `outs`), which compilers do not emit.

On the Steam Linux Runtime's `libstdc++` and `libgcc_s`, discovery adds less than
1% to FDE coverage that was already above 96%, and finds no `fs` access.

## What it means

- **The condition of the [TLS decision](TLS_UNDER_ROSETTA_DECISION.md) can be met.**
  Every `fs` access the build makes in proven code, 17,744 of them, has a located
  prefix and can be rewritten mechanically. No `fs` decode that could be a real
  access is left unexplained in any module.
- **What stays unproven is small and inert for T-001.** 8.1% of the code bytes is
  not reached by any rule. The only addressed `fs` decodes in it, 120, lie in
  blocks the code loads as data; the rest of it has none. If some of it is real
  code, such as functions reached only through jump tables, which discovery does not
  follow, that code makes no `fs` access the sweep can see. (INFERRED from the sweep
  of those bytes)
- **The load policy needs one precision.** The decision says a module with
  unexplained prefixes refuses to load. With discovery, the precise rule is: a
  module refuses to load when an unexplained decode could be a real access, that
  is, any kind but an `fs` string instruction. Data the code refers to and string
  instructions are logged, never rewritten.
- **The heuristics are measured, not proven sound.** A table that happened to decode
  as valid code without an overlap, an invalid byte, or an `fs` string instruction
  would be accepted as code. On this build, no accepted component contains an `fs`
  access other than the 10 real ones, so no data byte would be rewritten. The loader
  must recount per module at load time, and the counts here are the reference.
