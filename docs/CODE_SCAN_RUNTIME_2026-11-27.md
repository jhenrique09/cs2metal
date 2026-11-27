# Code scan of the Steam Linux Runtime, 27 November 2026

## Status

Measured on 27 November 2026. A first test of the `fs` to `gs` rewrite (transform
T-001) of the [TLS decision](TLS_UNDER_ROSETTA_DECISION.md) on real, publicly
distributed Linux code, before the game's own modules are available. Not edited
after the fact.

## Question

Does `cs2metal tls scan` find every `fs` access in real compiled code, and do the
`.eh_frame` unwind tables cover the code that contains those accesses well enough
for a decode-based rewrite?

## Method

The scan ran on five x86-64 libraries from Valve's public Steam Linux Runtime 3.0
("sniper", version 3.0.20261021, checksum verified): the C++ runtime that cs2metal
will load as guest code, and three glibc 2.31 libraries chosen because glibc uses
`fs` heavily and contains hand-written assembly. The glibc libraries were extracted
to a temporary directory for the measurement only; cs2metal does not load them.

Each count was checked against an independent tool: the number of instructions
that `objdump -d` prints with a `%fs:` operand.

## Results

| Library | Executable KB | Covered by FDEs | `fs` accesses found | `objdump` | Prefix located | Outside FDE ranges | Decoder failures |
| --- | --- | --- | --- | --- | --- | --- | --- |
| libstdc++ 6.0.28 | 879 | 96.2% | 0 | 0 | — | 0 | 0 |
| libgcc_s | 66 | 97.9% | 0 | 0 | — | 0 | 0 |
| libc 2.31 | 1,379 | 98.2% | 4,519 | 4,519 | 4,519 | 0 | 0 |
| libpthread 2.31 | 60 | 96.0% | 468 | 468 | 468 | 0 | 0 |
| libm 2.31 | 615 | 98.9% | 814 | 814 | 814 | 0 | 0 |

`fs` accesses by kind in glibc:

| Library | `fs:0` | `fs:0x28` (stack guard) | `fs:0x30` (pointer guard) | constant offset | register |
| --- | --- | --- | --- | --- | --- |
| libc 2.31 | 59 | 1,516 | 322 | 735 | 1,887 |
| libpthread 2.31 | 3 | 78 | 11 | 271 | 105 |
| libm 2.31 | 0 | 487 | 0 | 0 | 327 |

The scan of all five took under half a second.

All results are **CONFIRMED** on these files.

## Interpretation

- **The scanner is exact on real code.** Its counts agree with `objdump` on every
  library, and the `0x64` prefix byte was located in every access, so each one can
  be rewritten in place without changing an instruction's length. (CONFIRMED)
- **No `fs` access lies outside the unwind tables**, even in glibc, whose string,
  memory, and system call paths are hand-written assembly. The uncovered 2% to 4% of
  executable bytes are the PLT, `.init`, `.fini`, and alignment padding. This is the
  strongest evidence so far that T-001 is safe on compiled Linux code. Whether it
  holds for the game's bundled codec libraries, which carry their own assembly, is
  the next measurement. (CONFIRMED here; UNKNOWN for the game.)
- **The C++ runtime the guest will load touches `fs` nowhere.** libstdc++ reaches
  its thread-local data through `__tls_get_addr` (global-dynamic TLS), so it needs
  the loader's dynamic TLS support but no rewrite at all. (CONFIRMED)
- **glibc's numbers size the alternative libc routes.** Loading the runtime's real
  glibc would mean rewriting about 5,800 `fs` accesses across these three libraries
  and keeping glibc's thread control block layout exact, including the fields it
  reads at constant offsets (counted under "constant offset", which in glibc means
  control-block fields rather than local-exec variables). That is mechanical, but it
  is one more reason the libc bake-off of roadmap step C3 must be measured, not
  assumed. (INFERRED)

## Reproduce

```bash
cd <cs2metal checkout>
cargo run --release -p cs2metal -- tls scan ~/cs2metal-kit/sniper/files/lib/x86_64-linux-gnu
objdump -d --no-show-raw-insn <library> | grep -c '%fs:'
```
