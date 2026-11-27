# Decision: how the guest gets its C library

## Status

Proposed on 27 November 2026, with the measurements of the
[libc bake-off](LIBC_BAKEOFF_2026-11-27.md). **Waiting for the owner's choice.** This
is Gate 1 of the [roadmap](ROADMAP.md). Nothing past the bake-off is built until it
is decided.

## Context

The CS2 Linux build imports 682 strong glibc symbols and links against glibc 2.31
of the Steam Linux Runtime. The cs2metal loader can map, link, and run Linux code
under Rosetta 2, with TLS through `gs` (transform T-001) and a bridge between guest
and host domains. Something must now provide the C library. The
[architecture](ARCHITECTURE.md) planned an own ABI layer, and the
[route review](ROUTE_AND_EFFORT_REVIEW_2026-11-27.md) asked for that plan to be
compared with real glibc before the investment.

## Options

| Option | Summary | Measured cost | Main risk |
| --- | --- | --- | --- |
| A. Own glibc ABI layer | `cs2metal-glibc` in Rust: the 682 symbols, glibc's layouts translated at the boundary | About 430 of 682 functions cross the bridge or keep state; 33 to 197 libc calls per fixture, each non-neutral one paying a GS swap pair | Behavioral bugs across all of libc: stdio, locale, malloc, pthreads, signals |
| B. The runtime's glibc as it is, system calls trapped | Load glibc 2.31 unchanged and turn each `syscall` into a trap | 939 sites to rewrite (a second transform); about 34 µs per trapped call; `ud2` never returns | Performance: ruled out by measurement |
| C. glibc built from source with one system call slot | Rebuild glibc 2.31 with each `syscall` replaced by a call through `tcbhead_t.sysinfo`; the host answers Linux system calls | 20 source lines, then one more; a two-minute build; 970 of 975 call sites through the slot; only system calls cross, 8 to 36 per fixture | A Linux system call layer on macOS (futex, epoll, sockets, signals, `clone`, `mmap`), and glibc's coupling to ld.so |

## Proposal

**Option C, with glibc's own ld.so.**

- **The glibc build.** Build glibc 2.31 with the slot patch in the pinned sniper
  SDK, from Debian's `2.31-13+deb11u14` source for parity with the runtime, also
  patching `TLS_INIT_TP`. Build it in the fixture pipeline, with tracked checksums.
  Only the patch and the build script are tracked, never the binaries.
- **Starting a guest.** The host starts the rebuilt ld.so as the kernel would: maps
  it, builds the initial stack and auxiliary vector, and enters it with GS at a
  fresh control block. ld.so then loads `cs2` and its libraries itself.
- **System calls.** The host answers the slot with a Linux system call layer in
  Rust, growing from the 17 calls the fixtures use. `mmap` of an executable file
  range applies T-001 to the private mapping, so the transform stays
  registered, counted, and mechanical. `arch_prctl(ARCH_SET_FS)` sets the guest
  thread pointer.
- **Substitutions.** SDL3, Vulkan, and the Steam client library are still bridged
  by name, as the architecture describes. The bridging moves from the loader's
  binder to an ld.so audit hook or an `LD_PRELOAD`-style shim library, which is
  the first design question of the next slice.
- **The loader.** `cs2metal-loader` stays the home of T-001, the TLS and bridge
  primitives, and the fixture tests. Its symbol binding and relocation remain the
  fallback if option C meets a wall.

## Consequences

- C3's first slice becomes: the slot build in the fixture pipeline, then the host
  starting the rebuilt ld.so with `hello`, then the C and C++ fixtures with the
  runtime's libstdc++.
- [ARCHITECTURE.md](ARCHITECTURE.md) changes: the glibc ABI layer becomes a Linux
  system call layer, and the loader's role narrows.
- Distribution must offer the modified glibc source with the patch (LGPL).
- If the owner prefers option A, the loader built so far is its foundation as it
  stands, and the measurements here size its cost.
