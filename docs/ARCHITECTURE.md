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
├ cs2metal host: x86_64-apple-darwin process under Rosetta 2 ──────────────┤
│ integrity guard       baseline SHA-256 of the installation, checked      │
│ loader                map, relocate, bind, TLS, init order, dlopen       │
│ transform registry    T-001 fs→gs prefix rewrite (proposed)              │
│ bridge                GS swap between guest and host domains             │
│ glibc ABI layer       libc, libm, libpthread, libdl, librt in Rust       │
│ substitution table    SDL3 → macOS SDL3 · libvulkan → Vulkan loader      │
│                       steamclient → macOS steamclient (proposed)         │
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
- If `cs2` turns out to be a fixed-address `ET_EXEC` below 4 GB (open question Q-1),
  the host binary must be linked with a small `__PAGEZERO` so that range is free.

## Integrity guard

Before every launch the host hashes every file of the installation and compares the
result with the baseline the owner recorded after a clean Steam validation. Any
difference stops the launch and names the files. The baseline lives in the ignored
`baselines/` directory of the code repository and holds only relative paths.
`steamapps/` is excluded: it is the download tool's bookkeeping, not the game.

## Loader

It does what glibc's `ld.so` does, for the subset the inventory proves is used:

- Search order: the substitution table first, then bundled modules through
  `DT_RUNPATH` and `$ORIGIN`, then the glibc ABI layer. An import nothing provides
  binds to a stub that names the import when called (trap or log-and-return-zero).
- Symbol versioning, weak symbols, `STB_GNU_UNIQUE`, and interposition in global
  scope order, so a bundled allocator that exports `malloc` wins as it does on Linux.
- Relocations: the kinds the inventory counts, including `COPY`, `IRELATIVE`, the
  TLS relocations, and `RELR`.
- IFUNC resolvers run with the CPUID that Rosetta reports (no AVX advertised).
- TLS: a static TLS block per thread in glibc's variant II layout, a DTV,
  `__tls_get_addr`, and TLS descriptors.
- Initialization in dependency order (`DT_PREINIT_ARRAY`, `DT_INIT`, `DT_INIT_ARRAY`),
  finalization in reverse, `dlopen`, `dlsym`, `dladdr`, and `dl_iterate_phdr` so
  that libgcc_s can unwind C++ exceptions through `.eh_frame`.

## Transform registry

Every change to guest code or data beyond standard dynamic linking is listed here,
with its decision record and the counter the host logs at load time.

| Id | Transform | Decision | State |
| --- | --- | --- | --- |
| T-001 | Rewrite `fs` segment-override prefixes to `gs` in private mappings of executable segments | [TLS decision](TLS_UNDER_ROSETTA_DECISION.md) | Proposed |

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

## glibc ABI layer

A Rust implementation of the glibc symbols the inventory lists, with glibc's data
layouts on the guest side:

- Types whose layouts differ are translated at the boundary: `stat`, `dirent`,
  `sigaction` and `sigset_t`, `rlimit`, `utsname`, socket addresses and options,
  errno values, open, `mmap`, and `fcntl` flags, and signal numbers.
- pthread objects keep glibc's sizes. A guest mutex, condition variable, or
  read-write lock holds a pointer to a lazily created host object, so that glibc's
  all-zero static initializers keep working.
- Linux-only interfaces the game reads are synthesized: `/proc/self/exe`,
  `/proc/self/maps`, `/proc/cpuinfo`, `/proc/meminfo`, `getauxval`, `sysconf`.
- Process start: `__libc_start_main`, the auxiliary vector, `environ`, and the
  stack protector's guard values.

## Substitution table

Some libraries must not run as guest code because they talk to X11, Wayland, ALSA,
or the Linux kernel. Their imports bind instead to macOS libraries with the same C API:

| Linux library | Host replacement | Notes |
| --- | --- | --- |
| `libSDL3.so.0` | macOS SDL3, x86-64 build | Windows, input, audio, and Vulkan surface creation. The guest asks SDL for Vulkan instance extensions and receives the Metal surface ones. |
| `libvulkan.so.1` | Vulkan loader for macOS, x86-64 build | The driver is KosmicKrisp or MoltenVK (open question Q-6). |
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
| `cs2metal-loader` | Mapping, relocation, TLS, bridge (`unsafe` allowed) | M2 |
| `cs2metal-glibc` | glibc ABI layer | M3 |
| `cs2metal-platform` | SDL, Vulkan, and Steam substitutions | M5 to M7 |
