# The libc bake-off, 27 November 2026

## Status

Measured on 27 November 2026 for roadmap step C3, the first half of Gate 1: the
three ways of giving the guest a C library, compared on numbers before the large
investment. The choice itself belongs to the
[libc route decision](LIBC_ROUTE_DECISION.md), which the owner makes. Not edited
after the fact.

## Question

Which route gives the CS2 guest a working C library at the lowest cost and risk:
an own glibc ABI layer in Rust (A), the runtime's real glibc with its system calls
translated (B), or glibc built from source with a host backend (C)?

## Method

| Measure | How | Where |
| --- | --- | --- |
| Calls into glibc per fixture | `ltrace -f -c -l` on libc, libm, libpthread, libdl, librt, and ld.so, from every module | the fixtures, natively on the x86-64 Debian 13 virtual machine |
| Program system calls per fixture | `strace -f -k`, each call attributed to the innermost frame, so that ld.so's own calls are left out | the same |
| `syscall` instructions in the runtime's glibc | `objdump -d` on the sniper runtime's glibc 2.31 | the Mac |
| Cost of one trapped system call under Rosetta | `spikes/syscall-trap`: a signal handler emulates a call and resumes | the M1 Max, x86-64 under Rosetta 2 |
| glibc's dependence on ld.so internals | the `GLIBC_PRIVATE` imports of each sniper library | the Mac |
| How centralized glibc's system call layer is | `spikes/glibc-host-backend`: glibc 2.31 from its official source, every `syscall` replaced by a call through one slot, built in the pinned sniper SDK, then the raw instructions left counted | the virtual machine |
| What the game imports, by kind | the inventory's 682 strong glibc imports, classified by name | the Mac (INFERRED classification) |

A correction to the [inventory](LINUX_BUILD_INVENTORY_2026-11-27.md): its "710
distinct glibc symbols" counts (library, symbol) pairs, two of them weak only. The
distinct strong names are 682, because some appear in two libraries, as `malloc`
does in ld.so and libc.

The fixtures ran against the virtual machine's own glibc 2.41, not the runtime's
2.31. The call and system call counts are therefore orders of magnitude for this
code, not exact figures for the runtime. (Two system calls in the list, `clone3`
and `rseq`, are 2.41's.)

## Results

### Calls and system calls per fixture

| Fixture | Calls into glibc | Program system calls | ld.so system calls | Calls into libstdc++ |
| --- | --- | --- | --- | --- |
| hello | 71 | 8 | 28 | 1 |
| tls | 130 | 36 | 55 | 5 |
| stack_guard | 33 | 8 | 28 | 1 |
| cxx | 197 | 10 | 76 | 148 |
| dlopen | 78 | 8 | 95 | 1 |
| ifunc | 37 | 8 | 28 | 1 |
| scope | 50 | 8 | 63 | 1 |
| tlsprobe | 137 | 31 | 55 | 4 |

Calls into glibc outnumber the program's system calls by 4 to 20 times. Across all
fixtures the programs use 17 distinct system calls.

### Route B: the runtime's glibc as it is

| Library | `syscall` instructions | `fs` accesses |
| --- | --- | --- |
| libc 2.31 | 704 | 4,519 |
| libpthread 2.31 | 206 | 468 |
| librt 2.31 | 29 | 107 |
| libm, libdl | 0 | 814, 25 |

Under Rosetta 2 a Linux `syscall` runs as a macOS system call with the same number
(Linux `write` is 1, macOS `exit` is 1). Each of the 939 sites would need a second
transform. The only same-length replacement for a 2-byte instruction is a trap:

| Trap | Round trip under Rosetta 2 |
| --- | --- |
| `int3`, handled through `SIGTRAP` | about 34 µs (2000 rounds; 83 µs over 10 rounds) |
| `ud2`, handled through `SIGILL` | never returns from the handler |
| For scale: a plain call | 1.6 ns |
| For scale: a GS swap pair (segment probe) | 0.52 µs |

### What the runtime's glibc needs from ld.so

libc imports seven `GLIBC_PRIVATE` symbols from ld.so: `_rtld_global`,
`_rtld_global_ro`, `__libc_enable_secure`, `_dl_argv`, `_dl_exception_create`,
`__tunable_get_val`, and `_dl_find_dso_for_object`. libpthread imports
`_rtld_global`, `_rtld_global_ro`, and three TLS helpers from ld.so, plus 41 private
libc symbols, and libdl imports seven of ld.so's own. The first two are ld.so's
internal state structures, which libc reads field by field. A real glibc therefore
needs either glibc 2.31's ld.so data layout reproduced exactly, or the real ld.so.
This applies to routes B and C alike.

### Route C: glibc built with one system call slot

| Measure | Value |
| --- | --- |
| Source sites changed | 20: the 7 `INTERNAL_SYSCALL` templates and `DO_CALL` in `sysdep.h`, and 12 lines in 9 `.S` files |
| Build | succeeds unchanged otherwise, in the pinned sniper SDK (gcc 10.3), in 117 s on 8 vCPUs |
| Calls through the slot | libc 700, libpthread 205, librt 29, ld.so 36 |
| Raw `syscall` left | the three `__restore_rt` signal-return trampolines, and ld.so's `TLS_INIT_TP` (the `arch_prctl` that sets `fs`), one more source site |

Each slot call costs what any host call costs on the bridge, a GS swap pair, about
0.5 µs, plus the work of answering it.

### Route A: what an own layer must implement

| Kind of import | Symbols | Can run without crossing to the host |
| --- | --- | --- |
| System interface (files, time, signals, sockets, processes, dl) | 270 | no |
| String, memory, ctype, conversion | 159 | yes, written as neutral functions |
| Math (libm) | 91 | yes |
| stdio and formatting | 74 | partly: buffering, but every flush crosses |
| Threads and synchronization | 65 | no: futex-backed |
| Locale and internationalization | 17 | own state |
| Allocation | 6 | depends on the allocator written |
| Total | 682 | about 250 neutral, about 430 crossing or stateful |

(INFERRED: the classification is by symbol name. Every one of the 682 needs
glibc's exact behavior and layouts, which the
[route review](ROUTE_AND_EFFORT_REVIEW_2026-11-27.md) named the largest source of
bugs.)

## Interpretation

- **Route B is out.** Each system call would cost a trap of about 34 µs, about 65
  times a bridge crossing. A game makes thousands per second, and without a vDSO
  on the host even `clock_gettime` is one of them. The cheaper trap does not
  return at all under Rosetta. (CONFIRMED)
- **Route C is cheap where route B is expensive.** Twenty source lines send every
  system call of glibc but the signal trampolines through one slot, with no binary
  patching of glibc. The build takes two minutes in the toolchain the fixtures
  already use. (CONFIRMED)
- **Route C keeps the hot paths inside the guest.** With a real glibc, `malloc`,
  `memcpy`, string functions, stdio buffering, and pthread fast paths stay guest
  code, and only system calls cross: 8 to 36 per fixture, against 33 to 197 libc
  calls for route A. (CONFIRMED for the fixtures; for the game the ratio is
  expected to be larger.)
- **Route C moves the work, it does not remove it.** The host must answer Linux
  system calls with Linux semantics on macOS: about 17 for the fixtures, and for
  the game a larger set (futex, epoll, sockets, file systems, `mmap`, signals,
  `clone`), which the dedicated-server boot measures. Its glibc also needs ld.so's
  internals, so the natural form of route C runs the real, rebuilt ld.so too. The
  cs2metal host then starts ld.so as a kernel starts a process: it sets up the
  stack and auxiliary vector, answers its system calls, and applies T-001 to each
  executable file mapping it requests. Most of what the loader does today (symbol
  scopes, relocation, TLS layout) would then come from glibc itself. The loader
  stays as the T-001 and bridge foundation and as a test harness. (INFERRED)
- **Route A trades the system call layer for the whole C library.** Every one of
  682 functions must match glibc, and about 430 cross the bridge or keep state, on
  every call. It needs no glibc build and gives full control, which matters only if
  route C fails for a reason not yet seen.
- **Licensing is not an obstacle.** glibc is LGPL-2.1-or-later. A modified build
  shipped with a GPL-3.0 host is compatible if the modified source is offered with
  it. The project builds it from the official source and a tracked patch. The
  runtime's Debian 11 build also carries Debian's patches, so a parity build should
  start from Debian's `glibc 2.31-13+deb11u14` source rather than the upstream
  tarball. (REPORTED, licence terms)
