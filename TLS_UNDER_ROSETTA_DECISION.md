# Decision: thread-local storage under Rosetta

## Status

Proposed on 26 November 2026. Becomes accepted when the loader milestone (M2) runs
the project's own Linux fixtures through it and the owner signs off. Until then no
code implements the rewrite.

## Context

Linux x86-64 code addresses thread-local storage and the stack-protector canary
through the `fs` segment. The [probe](ROSETTA_SEGMENT_PROBE_2026-11-26.md) shows
that a process under Rosetta 2 cannot set an `fs` base by any known method, that an
`fs` access kills or hangs the process, and that the GS base can be moved and
restored at about 520 ns per pair.

The guest relies on the glibc thread control block layout: `fs:0` holds the block's
own address, `fs:0x10` the thread pointer, `fs:0x28` the stack guard and `fs:0x30`
the pointer guard. TLS variables sit at negative offsets from the thread pointer
(variant II), and dynamic TLS goes through `__tls_get_addr` or TLS descriptors.

## Options

| Option | Summary | Assessment |
| --- | --- | --- |
| 1. **Rewrite `fs` to `gs` in mapped guest code, swap GS at the boundary** | Change each `fs` segment-override prefix (`0x64`) to `gs` (`0x65`) in the loader's private mapping of executable segments. Guest threads run with the GS base at their Linux thread control block; every call into macOS code restores the host TSD base first and switches back on return. | Same instruction length, so no relocation of code. Build-independent: found by decoding, never by offsets. Costs a swap pair per host call that needs macOS thread data. |
| 2. Trap and emulate `fs` accesses | Let each access fault and emulate it | Impossible: the probe shows the access kills or hangs the process. |
| 3. Translate x86-64 to arm64 ourselves | Own the instruction translation so `fs` is just a register in memory | The only route that survives without Rosetta, and a project of its own. Kept for M9. |
| 4. Interpret guest code | An x86-64 interpreter | Orders of magnitude too slow for a game. |

## Proposal

Option 1, under these conditions:

- **Decode, never scan.** A byte-pattern search for `0x64` would corrupt data and
  instruction operands. Prefixes are found with a full x86-64 decoder over the
  function ranges that `.eh_frame` describes. Bytes outside known function ranges are
  decoded only when an independent rule proves they are code; otherwise they are
  reported, not rewritten.
- **Private copies only.** The rewrite applies to the loader's private, copy-on-write
  mapping. The file on disk stays byte-identical, and the integrity baseline proves
  it before each launch.
- **Registered and counted.** The transform is entry T-001 in the registry in
  [ARCHITECTURE.md](ARCHITECTURE.md). The host logs, per module, the number of
  instructions rewritten and the number of unexplained `fs` prefixes found outside
  decoded ranges. A module with unexplained prefixes refuses to load unless the
  owner overrides it explicitly.
- **Domain-aware bridge.** Host functions are classed as *neutral* (no macOS
  thread-specific data, such as `memcpy` implemented in Rust without libSystem
  calls), which skip the swap, or *host*, which swap. Callbacks from macOS into guest
  code (thread start routines, SDL callbacks, signal handlers, `qsort` comparators)
  swap the other way.
- **Signals.** A signal can arrive while GS points at a guest block. The host's
  signal entry must recognize that state, as Native VIctory's does by checking a
  known offset, and restore the host base before running any macOS code.

## Consequences

- This is an in-memory change to Valve's code. The owner has accepted that risk (see
  the [policy](ANTI_CHEAT_AND_INTEGRITY_POLICY.md)); it is also why the transform
  must stay mechanical, minimal, and visible.
- The cost of host calls sets a performance budget: about 0.5 µs per call that needs
  the swap. Hot paths (allocation, memory copies, string functions, locks) should be
  neutral. libSystem's own locks read thread data through GS, so neutral locks would
  have to sit directly on the kernel's `__ulock_wait` and `__ulock_wake` calls.
  (INFERRED; to be measured in M3.)
- Option 3 stays open as the long-term answer and as the exit from Rosetta.
