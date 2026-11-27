# Architecture

## Status

Adopted on 26 November 2026 and current as of 27 November 2026: every layer below
is built, and the game runs on it from its menu through a match
([record](CLIENT_PLAYABLE_2026-11-27.md)). It describes the design as built, and
changes with the code. The route itself is fixed by the
[Linux build decision](LINUX_BUILD_BASELINE_DECISION.md). Each bridge has its own
document: [SDL](SDL_BRIDGE.md), [Vulkan](VULKAN_BRIDGE.md), and
[Steam](STEAM_BRIDGE_DESIGN.md).

## Goal and non-goals

**Goal:** run the unmodified Linux build of Counter-Strike 2 on an Apple silicon Mac
inside one macOS process, with the game's code executing as itself and its graphics,
audio, input, and Steam calls answered by macOS.

**Non-goals:** modifying game files; anything that touches Valve Anti-Cheat; gameplay
changes of any kind; validating online play (going online is an owner decision,
not an engineering one, and what the owner reports of it is
[recorded](ONLINE_PLAY_2026-11-27.md), not measured); Intel Macs (nothing precludes
them, nothing targets them).

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
│ installation overlay  every write of the game lands outside it           │
├ macOS ───────────────────────────────────────────────────────────────────┤
│ libSystem · SDL 3 (Cocoa, CoreAudio) · MoltenVK on Metal · Steam client  │
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
difference stops the launch and names the files. The baseline is never tracked and holds only
relative paths: `install.sh` records it in `~/cs2-metal/data/baselines/`, and
`run.sh` records it again when steamcmd has installed a new build of the game. A
baseline recorded by hand defaults to the ignored `baselines/` directory of the
repository. `steamapps/` is excluded: it is the download tool's bookkeeping, not the
game.

## Process start and dynamic linking

By the [libc route decision](LIBC_ROUTE_DECISION.md), the guest runs glibc's own
dynamic loader, rebuilt with the rest of glibc 2.31. The host starts it as the Linux
kernel would:

- Map ld.so, build the initial stack (arguments, environment, auxiliary vector with
  `AT_PHDR`, `AT_ENTRY`, `AT_BASE`, `AT_PAGESZ`, `AT_RANDOM`, `AT_HWCAP`), and enter it
  with GS at a fresh control block.
- Make the vDSO and name it in the auxiliary vector (`AT_SYSINFO_EHDR`), as the
  kernel does: a one-page ELF object the host builds in memory, whose three
  symbols (`__vdso_clock_gettime`, `__vdso_gettimeofday`, `__vdso_time`, version
  `LINUX_2.6`) jump to neutral host functions. glibc finds them as it does on
  Linux and reads the clocks without a system call (see Time, below).
- ld.so then does everything `ld.so` does: it opens and maps `cs2` and every library
  through system calls, relocates, binds with versions, runs IFUNC resolvers, lays out
  TLS, and later serves `dlopen`. Each of those system calls reaches the host.
- When ld.so maps an executable range of a file, the host applies T-001 to that
  private mapping before returning, using the code scan's proof for the whole file.
- Libraries in the substitution table are not loaded as guest code. Wherever the
  guest opens one of them, the system call layer answers with the project's shim
  of the same name instead: a small Linux library that exports the same functions
  and forwards each one through a table the host fills with its replacements. The
  game's own file is neither read nor changed.

`cs2metal-loader`, the project's own linker, stays as the home of T-001, the TLS
and bridge primitives, and the fixture tests, and as the fallback should this route
meet a wall.

## Transform registry

Every change to guest code or data beyond standard dynamic linking is listed here,
with its decision record and the counter the host logs at load time.

| Id | Transform | Decision | State |
| --- | --- | --- | --- |
| T-001 | Rewrite `fs` segment-override prefixes to `gs` in private mappings of executable segments | [TLS decision](TLS_UNDER_ROSETTA_DECISION.md) | Accepted. Applied to every executable file mapping of a game boot; counted per module (`FsReport`: rewritten, in data, unexplained) |

## Execution domains and the bridge

- **Guest domain:** the GS base points at the thread's Linux control block (glibc
  layout: self pointer at 0, thread pointer at 0x10, stack guard at 0x28, pointer
  guard at 0x30).
- **Host domain:** the GS base points at the macOS TSD array of the thread.
- Every bound host function is reached through a thunk. A *host* thunk switches to
  the host domain for the call and back on return. A *neutral* function touches no
  macOS thread data and is called directly: it reads nothing through GS, takes no
  lock of libSystem, allocates nothing, and cannot panic. The neutral functions
  are `__tls_get_addr`, `__stack_chk_fail`, the clocks behind the vDSO, the
  system call entry's fast handler, and the functions that record Vulkan
  commands ([Vulkan bridge](VULKAN_BRIDGE.md)). That a
  function is neutral is tested, not assumed: `probe_neutral` runs it with GS at
  a block of poison words, which following stops the process and writing is
  reported.
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

Before it switches, the entry asks a *fast handler*, in the guest's domain. The
calls that need none of the host's thread data are answered there, with no GS
switch: `futex` wait and wake, which go straight to the kernel (one trap where
there were three), `sched_yield`, and `clock_gettime`, `gettimeofday`, and `time`
for a caller that does not go through the vDSO. The handler is neutral code,
tested with GS at a block of poison words on its error paths too. It declines
whatever needs more, and every call while `CS2METAL_TRACE` or
`CS2METAL_FUTEX_TRACE` is on; `CS2METAL_NO_FAST_SYSCALLS=1` turns it off.

- Numbers, flags, structures, and errno values are Linux's on the guest side and are
  translated at the boundary: `stat`, `dirent64`, `sigaction`, socket addresses,
  `open`, `mmap`, and `fcntl` flags, signal numbers.
- Threads: `clone` with glibc's flags becomes a host thread whose GS starts at the new
  thread's control block. Every guest thread, the main one too, asks macOS for
  the user-interactive class, which keeps it on the performance cores ahead of
  background work and its timers exact (`CS2METAL_NO_QOS=1` leaves them as macOS
  makes them); `futex` becomes the kernel's wait and wake on an address (`__ulock_wait2`
  and `__ulock_wake`, what `os_sync_wait_on_address` is made of), asked not to
  set `errno`, so that it is neutral;
  `set_robust_list` and `rseq` are recorded or declined as glibc tolerates.
  `CS2METAL_FUTEX_TRACE=<seconds>` reports, at that period, each wait parked for two
  seconds or more (its word, the value waited against and the value held now, and
  the guest code that made the call) and the most woken words.
- Memory: `mmap`, `munmap`, `mprotect`, `brk`, and `madvise`, with T-001 at executable
  file mappings.
- Linux-only files and calls the game reads are synthesized: `/proc/self/exe`,
  `/proc/self/maps`, `/proc/cpuinfo`, `/proc/meminfo`, `getrandom`, `uname`,
  `prlimit64`.
- The processors the guest is told of, in `/proc/cpuinfo`, under
  `/sys/devices/system/cpu`, and by `sched_getaffinity`, are the Mac's performance
  cores only: eight on an M1 Max, not ten. The engine sizes its job pool by that
  number and splits a frame's work evenly over it, so a job thread on an
  efficiency core would hold the frame. `CS2METAL_CPUS=<n>` names another number.
- Open files: a process that launchd starts, as an application is, has a soft
  limit of 256 open files, and so has one started from a shell that did not raise
  it. The game keeps more than 500 of its package files open, and stops with a
  fatal error where it cannot open one. The host raises the soft limit to 10240
  before the guest starts, and `prlimit64` answers with the limit in force.
- The installation overlay: the game writes into its own installation
  (configuration, logs, caches). No write reaches it. Every guest path inside the
  installation is looked up first in an overlay directory outside it; a write goes
  to the overlay, copying the original up first when its contents are kept, and
  removing an original file is refused with `EROFS`. Reads of anything the game
  has not written fall through to the installation, which stays byte-identical to
  its baseline.
- A stand-in for the opening movie: with `CS2METAL_NO_INTRO` set, a read of one of
  the game's opening movies (the `intro` files of its Panorama videos) is answered
  with the project's own file, half a second of black frames with silent sound in
  the same formats. The game ignores `-novid` in this build, and shows an error on
  screen when the movie is missing; given the stand-in it shows a black screen for
  the three and a half seconds until its menu is ready (client boot 79 of 27
  November 2026). The game's file is not touched.
- Signals: the host's own handlers receive macOS signals and deliver Linux frames to
  the guest's handlers; the guest's `__restore_rt` trampolines, the only raw `syscall`
  left in the rebuilt glibc, are never run.
  Not built yet: the guest's handlers are recorded and never called. Meanwhile a
  fatal signal (SIGILL, SIGTRAP, SIGABRT, SIGFPE, SIGBUS, SIGSEGV) writes one line to
  standard error before the process ends: the faulting instruction, the registers,
  and the return addresses the frame pointers lead to, which `CS2METAL_MAPS=1` lets
  one read as guest modules and offsets.
  `CS2METAL_STACKS=<seconds>` asks every guest thread for the same line at that
  period, which shows where a thread that never returns is, with its registers.
  SIGTERM and SIGINT are the exception that is delivered: they become SDL's quit
  event, so the game shuts itself down as it does from its menu.
- Time: the guest reads its clocks through the vDSO, in its own domain, with no
  system call and no GS switch. Before the vDSO, in a match, the game's main
  thread spent 13% of its processor time in `clock_gettime` system calls
  ([measurements](PERFORMANCE_AB_2026-11-27.md)). One function answers every
  reader of a clock (the vDSO, the `clock_gettime`, `gettimeofday`, and `time`
  system calls, and the absolute deadlines of `futex` and `clock_nanosleep`), so
  a deadline is compared with the clock it was computed from. The monotonic
  clocks are `mach_absolute_time`, `CLOCK_BOOTTIME` is `mach_continuous_time`,
  and the wall clock is `gettimeofday`. The processor-time clocks need the
  thread's own state: the vDSO answers `-ENOSYS` for them, and glibc makes the
  system call. `CS2METAL_NO_VDSO=1` starts the guest without the vDSO.

## Substitution table

Some libraries must not run as guest code because they talk to X11, Wayland, ALSA,
or the Linux kernel. Their imports bind instead to macOS libraries with the same C API:

| Linux library | Host replacement | Notes |
| --- | --- | --- |
| `libSDL3.so.0` | macOS SDL3, x86-64 build | Windows, input, audio, and Vulkan surface creation. The guest asks SDL for Vulkan instance extensions and receives the Metal surface ones. |
| `libvulkan.so.1` | Vulkan loader for macOS, x86-64 build | Commands recorded into a command buffer do not cross the bridge one by one: they are recorded on the guest's side and replayed in one crossing. The driver is MoltenVK: the only one that loads in an x86-64 process (Q-6, answered on 27 November 2026 by the [Vulkan driver measurement](VULKAN_DRIVER_MEASUREMENT_2026-11-27.md)). |
| `steamclient.so` | `steamclient.dylib` of the owner's macOS Steam installation | The game's own `libsteam_api.so` stays guest code; only the client library it loads is bridged, with proxied interface objects. See the [Steam bridge design](STEAM_BRIDGE_DESIGN.md). |

The C++ runtime is the opposite case: Linux libstdc++ and libgcc_s are loaded **as
guest code** from the owner's copy of the Steam Linux Runtime, because macOS libc++
does not share their data layouts.

## Diagnostics

The host is quiet unless asked. Each of these is an environment variable; what it
reports goes to standard error, which `run.sh` writes to the session's log.

| Variable | Reports |
| --- | --- |
| `CS2METAL_MAPS=1` | Each guest module as it is mapped, with its address range, so that a guest address can be named as a module and an offset |
| `CS2METAL_TRACE=1` | Every system call the guest makes |
| `CS2METAL_FUTEX_TRACE=<seconds>` | At that period: waits parked for two seconds or more, and the most woken words |
| `CS2METAL_STACKS=<seconds>` | At that period: where every guest thread is, with its registers |
| `CS2METAL_SDL_TRACE=1` | The events the game takes, its input and audio calls, the guest code that made them, and each event pump or event read of 3 ms or more with what took the time ([SDL bridge](SDL_BRIDGE.md)) |
| `CS2METAL_VULKAN_TRACE=1` | Counts of Vulkan calls, the pacing of presents every ten seconds, and each gap of a quarter second or more ([Vulkan bridge](VULKAN_BRIDGE.md)) |
| `CS2METAL_SHADER_TRACE=1` | MoltenVK's own timings of each step of making a pipeline, of encoding command buffers, and of the GPU, every 3000 frames ([Vulkan bridge](VULKAN_BRIDGE.md)) |
| `CS2METAL_STEAM_TRACE=1` | The Steam client interfaces the game asks for, by version ([Steam bridge](STEAM_BRIDGE_DESIGN.md)) |

Always on:

- A fatal signal writes one line: the faulting instruction, the registers, and the
  return addresses.
- Every registered transform is counted at load.
- Each bridge says, as it binds, how many of the shim's functions the host library
  has (`bound N of M`).

Reports that name guest symbols stay local, in the ignored `runs/` and `reports/`
directories: they contain Valve's symbol names.

## Rosetta horizon

The host runs only as long as Rosetta does. Apple states that macOS 27 is the last
general-purpose Rosetta release. By the [Rosetta horizon decision](ROSETTA_HORIZON_DECISION.md),
the project accepts that limit as a research prototype. Rosetta-specific mechanisms
(the code transforms and the GS bridge) stay behind one module boundary of the
loader, every other layer is written so that it does not depend on Rosetta, and the
private facts the bridge relies on are discovered at run time, never hard-coded.

## Crate map

| Crate | Role |
| --- | --- |
| `cs2metal` (app) | The command-line program: `run`, `baseline`, `inspect`, `inventory`, `tls`, `vulkan` |
| `cs2metal-elf` | Read-only, bounds-checked ELF64 parser |
| `cs2metal-inventory` | Installation scan and host requirements |
| `cs2metal-integrity` | Baseline and verification |
| `cs2metal-codescan` | `fs` accesses, unwind coverage, TLS models; the proof T-001 relies on |
| `cs2metal-vulkan` | Static scan of the renderer's Vulkan requirements |
| `cs2metal-loader` | Mapping, relocation, TLS, T-001, the GS bridge, and the system call entry (`unsafe` allowed) |
| `cs2metal-linux` | Process start, the Linux system call layer, the installation overlay, and the SDL, Vulkan, and Steam bridges (`unsafe` allowed) |

Outside the crates, `guest/glibc` holds the guest C library's patch and build, and
`guest/shims` the three shim libraries.
