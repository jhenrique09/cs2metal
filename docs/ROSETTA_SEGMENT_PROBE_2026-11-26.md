# Rosetta segment-register probe, 26 November 2026

## Status

Measured on 26 November 2026. Feeds [the TLS decision](TLS_UNDER_ROSETTA_DECISION.md).
Not to be edited after the fact; newer measurements get a new dated document.

## Question

Linux x86-64 code reaches its thread-local storage through the `fs` segment: glibc
points the `fs` base at each thread's control block with `arch_prctl(ARCH_SET_FS)`,
compilers emit `mov %fs:offset` for initial-exec and local-exec TLS, and every
function built with the stack protector reads its canary from `%fs:0x28`. Can an
x86-64 process under Rosetta 2 on macOS set an `fs` base at all?

## Method

`spikes/rosetta-segments/probe.c` in the code repository, run by
`spikes/rosetta-segments/run.sh`. Each case runs in a separate process with a
five-second timeout, because several cases never return.

Machine: Apple M1 Max, macOS 27.2.1 (build 26B131), Rosetta 2 as shipped.

## Results

| Case | Outcome | Detail |
| --- | --- | --- |
| `cpuid` | ok | Vendor `GenuineIntel`, maximum leaf 13. SSE4.2 reported. AVX, F16C, FSGSBASE, AVX2 and BMI2 **not** reported. XCR0 = `0x7` (x87, SSE and AVX state enabled). |
| `avx2` | ok | AVX2 instructions execute although CPUID does not advertise AVX2. |
| `rdfsbase` | hang | The process never returns. No signal reaches the process. |
| `wrfsbase` | hang | Same as `rdfsbase`. |
| `fs-read` | SIGSEGV | The `fs` selector is 0, and `mov %fs:0` kills the process with SIGSEGV. With a SIGSEGV handler installed, an earlier run of the same access hung instead. |
| `ldt-auto` | abort | Rosetta prints `i386_set_ldt(kLdtAutoAlloc) not supported` and aborts. |
| `ldt-fixed` | abort | Rosetta prints `LDT ... not supported` and aborts. |
| `gs-swap` | ok | `thread_fast_set_cthread_self` (`0x3000003`) moves the GS base; `gs:8` reads the probe's block. Restoring the host TSD base (found at `pthread_self() + 0xE0`) leaves `pthread_getspecific` working. A swap pair (set, restore) costs **520 ns** on this M1 Max, averaged over one million pairs. |

All outcomes are **CONFIRMED** on this machine and macOS build.

## Interpretation

- **There is no way to give Linux guest code an `fs` base under Rosetta.** The
  FSGSBASE instructions hang, segment descriptors are refused, and macOS has no
  `arch_prctl`. (CONFIRMED for the methods tried; no other method is known.)
- **An unhandled `fs` access is fatal and an `fs` access under a SIGSEGV handler can
  hang.** The loader cannot emulate `fs` accesses by trapping them. (CONFIRMED)
- **`gs` is usable, as Native VIctory for macOS found for Windows code.** Guest code
  whose `fs` accesses are rewritten to `gs` can run with the GS base pointing at a
  Linux-layout thread control block, provided the host restores its own TSD base
  before any macOS code runs. (CONFIRMED for the mechanism; INFERRED for CS2.)
- **The swap is expensive.** At 520 ns per pair, a call into macOS that needs the
  host GS base costs at least half a microsecond more than a plain call. Calls that
  touch no thread-specific data can skip the swap. (CONFIRMED cost; INFERRED policy.)
- **CPUID understates the CPU.** Guest code that dispatches on CPUID (glibc-style
  IFUNC resolvers, engine CPU detection) will choose SSE4.2 paths even though AVX2
  executes. That is safe, and it also avoids the AVX masked-store faults that Native
  VIctory found Rosetta cannot resume. (CONFIRMED CPUID values; INFERRED effect.)

## Reproduce

```bash
cd <cs2metal checkout>
spikes/rosetta-segments/run.sh
```
