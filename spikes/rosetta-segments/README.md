# Rosetta segment-register probe

Answers one question for the loader: can a Rosetta 2 process point `fs` at a Linux
thread control block, as glibc does with `arch_prctl(ARCH_SET_FS)`? Linux x86-64
code reaches its thread-local storage, and the stack-protector canary at `fs:0x28`,
through `fs`. macOS uses `gs` for its own thread-specific data.

```bash
spikes/rosetta-segments/run.sh
```

The script builds `probe.c` for x86-64 into `target/spikes/` and runs each case in
its own process with a five-second timeout (`PROBE_TIMEOUT` overrides it).

| Case | What it tries |
| --- | --- |
| `cpuid` | CPU features Rosetta reports: SSE4.2, AVX, F16C, FSGSBASE, AVX2, BMI2, XCR0 |
| `avx2` | Executes AVX2 instructions whatever CPUID says |
| `rdfsbase`, `wrfsbase` | The FSGSBASE instructions |
| `fs-read` | A plain `mov %fs:0` with the default `fs` |
| `ldt-auto`, `ldt-fixed` | A data segment from `i386_set_ldt`, as 32-bit Wine once used |
| `gs-swap` | Sets the GS base with `thread_fast_set_cthread_self`, reads through it, restores the host TSD base, checks `pthread_getspecific`, and times a swap pair |

The results, their interpretation, and the decision they feed are recorded in the
documentation repository: `docs/ROSETTA_SEGMENT_PROBE_2026-11-26.md` and
`docs/TLS_UNDER_ROSETTA_DECISION.md`.
