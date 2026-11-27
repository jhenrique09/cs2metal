# Architecture

## Status

Normative for milestones M0 to M2, adopted on 26 November 2026. Later sections are
the current design and change as the milestones in the
[implementation plan](IMPLEMENTATION_PLAN.md) produce evidence. The route itself is
fixed by the [Linux build decision](LINUX_BUILD_BASELINE_DECISION.md).

## Goal and non-goals

**Goal:** run the unmodified Linux build of Counter-Strike 2 on an Apple silicon Mac
inside one macOS process, with the game's code executing as itself and its graphics,
audio, input, and Steam calls answered by macOS.

**Non-goals:** modifying game files; anything that touches Valve Anti-Cheat; gameplay
changes of any kind; online competitive validation (an owner decision, not an
engineering one); Intel Macs (nothing precludes them, nothing targets them).

## Layers

```
┌ CS2 Linux build: ELF x86-64, byte-identical on disk ─────────────────────┐
│ cs2 · libtier0 · libengine2 · libclient · librendersystemvulkan · ...    │
│ + guest runtime: Linux libstdc++ and libgcc_s (owner-supplied)           │
│ + guest C library: glibc 2.31 and its ld.so, rebuilt with one system     │
│   call slot (built by the project from source)                           │
├ cs2metal host: x86_64-apple-darwin process under Rosetta 2 ──────────────┤
│ integrity guard       baseline SHA-256 of the installation, checked      │
│ process start         maps ld.so, builds the stack and auxiliary vector  │
│ system call layer     Linux system calls answered with macOS services    │
│ transform registry    T-001 fs→gs prefix rewrite, applied at mmap        │
│ bridge                GS swap between guest and host domains             │
│ substitution table    SDL3 → macOS SDL3 · libvulkan → Vulkan loader      │
│                       steamclient → macOS steamclient                    │
├ macOS ───────────────────────────────────────────────────────────────────┤
│ libSystem · Vulkan on Metal (KosmicKrisp or MoltenVK) · CoreAudio · HID  │
└ Apple silicon ───────────────────────────────────────────────────────────┘
```

The key enabling fact: Linux and macOS share the System V AMD64 calling convention
and the Itanium C++ ABI on x86-64, so a guest call reaches a host function directly
once its arguments have the same layout. The two costs every crossing can carry are
the thread-pointer swap and the translation of data layouts that differ.

## Process and address space

- One process, built for `x86_64-apple-darwin`, running under Rosetta 2. Guest and
  host share the address space.
- Guest modules are mapped from read-only file descriptors into private mappings.
  Shared objects are position-independent and go wherever the loader puts them.
- `cs2` is a position-independent executable (Q-1, answered on 27 November 2026 by
  the [inventory](LINUX_BUILD_INVENTORY_2026-11-27.md)), so it too goes wherever the
  loader puts it, and the host keeps the default `__PAGEZERO`.
- Pages are 4 KiB in a Rosetta 2 process, which matches the 4 KiB segment alignment
  of Linux x86-64 objects, so modules are mapped straight from their files. A native
  arm64 process has 16 KiB pages and could not map them. The loader refuses with a
  typed error, as its tests show on both architectures. (CONFIRMED, 27 November 2026)

## Integrity guard

Before every launch the host hashes every file of the installation and compares the
result with the baseline the owner recorded after a clean Steam validation. Any
difference stops the launch and names the files. The baseline lives in the ignored
`baselines/` directory of the code repository and holds only relative paths.
`steamapps/` is excluded: it is the download tool's bookkeeping, not the game.

## Process start and dynamic linking

By the [libc route decision](LIBC_ROUTE_DECISION.md), the guest runs glibc's own
dynamic loader, rebuilt with the rest of glibc 2.31. The host starts it as the Linux
kernel would:

- Map ld.so, build the initial stack (arguments, environment, auxiliary vector with
  `AT_PHDR`, `AT_ENTRY`, `AT_BASE`, `AT_PAGESZ`, `AT_RANDOM`, `AT_HWCAP`), and enter it
  with GS at a fresh control block.
- ld.so then does everything `ld.so` does: it opens and maps `cs2` and every library
  through system calls, relocates, binds with versions, runs IFUNC resolvers, lays out
  TLS, and later serves `dlopen`. Each of those system calls reaches the host.
- When ld.so maps an executable range of a file, the host applies T-001 to that
  private mapping before returning, using the code scan's proof for the whole file.
- Libraries in the substitution table are not loaded as guest code. The guest
  receives a stub library whose functions enter the host's replacements. How the
  stubs are presented to ld.so (an audit hook, or a preloaded shim library) is the
  first design question of that work.

`cs2metal-loader`, the project's own linker, stays as the home of T-001, the TLS
and bridge primitives, and the fixture tests, and as the fallback should this route
meet a wall.

## Transform registry

Every change to guest code or data beyond standard dynamic linking is listed here,
with its decision record and the counter the host logs at load time.

| Id | Transform | Decision | State |
| --- | --- | --- | --- |
| T-001 | Rewrite `fs` segment-override prefixes to `gs` in private mappings of executable segments | [TLS decision](TLS_UNDER_ROSETTA_DECISION.md) | Accepted. Implemented behind `FsRewrite`, off by default; counted per module (`FsReport`: rewritten, in data, unexplained) |

## Execution domains and the bridge

- **Guest domain:** the GS base points at the thread's Linux control block (glibc
  layout: self pointer at 0, thread pointer at 0x10, stack guard at 0x28, pointer
  guard at 0x30).
- **Host domain:** the GS base points at the macOS TSD array of the thread.
- Every bound host function is reached through a thunk. A *host* thunk switches to
  the host domain for the call and back on return. A *neutral* function touches no
  macOS thread data and is called directly.
- Calls from macOS into guest code (thread start routines, SDL callbacks, signal
  handlers, sort comparators, `atexit` handlers) switch the other way.
- The signal entry detects which domain was interrupted and restores the host
  domain before running any macOS code.

## Linux system call layer

The guest's glibc issues every system call through one slot, `tcbhead_t.sysinfo` at
`fs:0x20` (rewritten to `gs:0x20` by T-001), which points at a host entry. The entry
preserves every register a Linux `syscall` preserves, switches to the host domain,
answers the call with Linux semantics, and returns the result or `-errno` in `rax`.
It keeps a standard frame record (the guest's `rbp` under the return address), so a
host stack sample of a thread inside a system call continues into the guest code
that made it.

- Numbers, flags, structures, and errno values are Linux's on the guest side and are
  translated at the boundary: `stat`, `dirent64`, `sigaction`, socket addresses,
  `open`, `mmap`, and `fcntl` flags, signal numbers.
- Threads: `clone` with glibc's flags becomes a host thread whose GS starts at the new
  thread's control block; `futex` becomes `__ulock_wait` and `__ulock_wake`;
  `set_robust_list` and `rseq` are recorded or declined as glibc tolerates.
- Memory: `mmap`, `munmap`, `mprotect`, `brk`, and `madvise`, with T-001 at executable
  file mappings.
- Linux-only files and calls the game reads are synthesized: `/proc/self/exe`,
  `/proc/self/maps`, `/proc/cpuinfo`, `/proc/meminfo`, `getrandom`, `uname`,
  `prlimit64`.
- Signals: the host's own handlers receive macOS signals and deliver Linux frames to
  the guest's handlers; the guest's `__restore_rt` trampolines, the only raw `syscall`
  left in the rebuilt glibc, are never run.
  Not built yet: the guest's handlers are recorded and never called. Meanwhile a
  fatal signal (SIGILL, SIGTRAP, SIGABRT, SIGFPE, SIGBUS, SIGSEGV) writes one line to
  standard error before the process ends: the faulting instruction, the registers,
  and the return addresses the frame pointers lead to, which `CS2METAL_MAPS=1` lets
  one read as guest modules and offsets.
- Time: there is no vDSO, so `clock_gettime` and `gettimeofday` are system calls,
  answered with the host clock and counted for performance.

## Substitution table

Some libraries must not run as guest code because they talk to X11, Wayland, ALSA,
or the Linux kernel. Their imports bind instead to macOS libraries with the same C API:

| Linux library | Host replacement | Notes |
| --- | --- | --- |
| `libSDL3.so.0` | macOS SDL3, x86-64 build | Windows, input, audio, and Vulkan surface creation. The guest asks SDL for Vulkan instance extensions and receives the Metal surface ones. |
| `libvulkan.so.1` | Vulkan loader for macOS, x86-64 build | The driver is MoltenVK: the only one that loads in an x86-64 process (Q-6, answered on 27 November 2026 by the [Vulkan driver measurement](VULKAN_DRIVER_MEASUREMENT_2026-11-27.md)). |
| `steamclient.so` | `steamclient.dylib` of the owner's macOS Steam installation | The game's own `libsteam_api.so` stays guest code; only the client library it loads is bridged, with proxied interface objects. See the [Steam bridge design](STEAM_BRIDGE_DESIGN.md). |

The C++ runtime is the opposite case: Linux libstdc++ and libgcc_s are loaded **as
guest code** from the owner's copy of the Steam Linux Runtime, because macOS libc++
does not share their data layouts.

## Diagnostics

- Every unresolved import is named when first called.
- Crash reports symbolize guest addresses against the guest modules' dynamic symbols.
  They stay local: they contain Valve symbol names.
- Load-time counters for every registered transform.

## Rosetta horizon

The host runs only as long as Rosetta does. Apple states that macOS 27 is the last
general-purpose Rosetta release. By the [Rosetta horizon decision](ROSETTA_HORIZON_DECISION.md),
the project accepts that limit as a research prototype. Rosetta-specific mechanisms
(the code transforms and the GS bridge) stay behind one module boundary of the
loader, every other layer is written so that it does not depend on Rosetta, and the
private facts the bridge relies on are discovered at run time, never hard-coded.

## Crate map

| Crate | Role | State |
| --- | --- | --- |
| `cs2metal-elf` | Read-only ELF64 parser | M0, done |
| `cs2metal-inventory` | Installation scan and host requirements | M0, done |
| `cs2metal` (app) | Command-line front end | M0, `inspect` and `inventory` |
| `cs2metal-integrity` | Baseline and verification | M1, done |
| `cs2metal-codescan` | `fs` accesses, unwind coverage, TLS models | M1, done |
| `cs2metal-loader` | Mapping, relocation, TLS, T-001, the GS bridge (`unsafe` allowed) | M2, three slices: one module, a symbol scope, TLS and the bridge |
| `cs2metal-linux` | Process start and the Linux system call layer (`unsafe` allowed) | M3: process start, files, memory with T-001, threads, futex; all eight fixtures pass |
| `cs2metal-platform` | SDL, Vulkan, and Steam substitutions | M5 to M7 |
