# The CS2 Linux build, inventoried and scanned, 27 November 2026

## Status

Measured on 27 November 2026 on build 26879581 of app 730, downloaded to a Mac by
steamcmd. Answers open questions Q-1 to Q-5 of the
[feasibility research](FEASIBILITY_RESEARCH_2026-11-26.md) and records the first
measurement of the `fs` rewrite (T-001) on the game's own modules, for roadmap steps
A4, B2, B3, and B5. Not edited after the fact; later builds get a new document.

The detailed reports stay in the ignored `reports/` directory. This document cites
counts, public ABI names (glibc, the C++ runtime, the Steam Linux Runtime), and file
names of the installation, never Valve's own symbols or strings.

## Method

```bash
cargo run --release -p cs2metal -- baseline record ~/cs2metal-kit/cs2-linux
cargo run --release -p cs2metal -- baseline verify ~/cs2metal-kit/cs2-linux --quick
cargo run --release -p cs2metal -- inventory ~/cs2metal-kit/cs2-linux \
  --markdown reports/inventory.md --json reports/inventory.json
cargo run --release -p cs2metal -- inspect ~/cs2metal-kit/cs2-linux/game/bin/linuxsteamrt64/cs2
cargo run --release -p cs2metal -- tls scan ~/cs2metal-kit/cs2-linux \
  --markdown reports/tls-scan.md --json reports/tls-scan.json
```

Every count in the code scan was checked against an independent tool: `objdump -d`
for `fs` operands, and the FDE count in each module's `.eh_frame_hdr` for the
unwind-table parser.

## A4: steamcmd delivers the Linux build to a Mac

The platform override works from macOS. steamcmd on an M1 Max installed app 730 with
the Linux depot: `StateFlags` 4 (fully installed), 71.1 GB on disk, and
`game/bin/linuxsteamrt64/cs2` is an x86-64 ELF executable. The step that was the
first experiment of the roadmap succeeded. (CONFIRMED)

## B3: integrity baseline

| Measure | Value |
| --- | --- |
| Files recorded | 3,131 |
| Symbolic links | 0 |
| Bytes hashed | 71.1 GB |
| Time to record, all cores, files just written | 11 s |
| Differences on the first `verify --quick` | 0 |

The installation has no symbolic links, so every file the loader opens is a plain
file covered by the baseline.

## Q-1: the executable

`cs2` is a position-independent executable (`ET_DYN` with an interpreter), not a
fixed-address `ET_EXEC`. Risk R-9 of the plan does not apply: the host needs no small
`__PAGEZERO`, and the executable can be mapped at any free address.

It is a small launcher: about 100 KB of code, 49 imports, all from glibc
(`libc`, `libdl`, `libpthread`, and `ld-linux-x86-64.so.2`), one `COPY` relocation,
`DT_BIND_NOW`, and a 16-byte TLS block. Its search path is `$ORIGIN`. It needs
`libdl`, so the engine modules are reached by `dlopen` rather than by `DT_NEEDED`
(INFERRED from the imports; observed at run time in C4).

## Q-2: what the build bundles and what it expects

61 ELF modules, all x86-64, all parse; none has text relocations. 60 are shared
objects in `game/bin/linuxsteamrt64` and `game/csgo/bin/linuxsteamrt64`, including
SDL3, FFmpeg 4 (`libavcodec.so.58` and siblings), Pango, Cairo, FreeType,
Fontconfig, libvpx, Vorbis, Ogg, mpg123, V8 with ICU, `libsteam_api.so`, and both
`librendersystemvulkan.so` and `librendersystemempty.so`.

They expect 21 libraries from the system, which is the Steam Linux Runtime:

| Family | Libraries | Modules needing them |
| --- | --- | --- |
| glibc | `libc`, `libm`, `libpthread`, `libdl`, `librt`, `ld-linux-x86-64` | up to 61 |
| GCC runtime | `libstdc++.so.6`, `libgcc_s.so.1` | 2 each |
| FFmpeg's hardware back ends | `libva`, `libvdpau`, `libdrm`, `libX11` | 1 or 2, only FFmpeg |
| Text and data | `libglib-2.0`, `libgobject-2.0`, `libharfbuzz`, `libfribidi`, `libthai`, `libexpat`, `libuuid`, `libz`, `libbz2` | 1 to 3 |

Only FFmpeg's `libavutil` and `libavcodec` reach for the window system and video
acceleration libraries; the engine itself names no X11, Wayland, or Vulkan loader
through `DT_NEEDED`. The Vulkan loader is opened at run time (INFERRED).

## Q-3: the glibc surface

710 distinct glibc symbols, non-weak, across six libraries:

| Library | Symbols | Newest version required |
| --- | --- | --- |
| `libc.so.6` | 514 | GLIBC_2.29 |
| `libm.so.6` | 91 | GLIBC_2.29 |
| `libpthread.so.0` | 89 | GLIBC_2.30 |
| `libdl.so.2` | 7 | GLIBC_2.3.4 |
| `ld-linux-x86-64.so.2` | 6 | GLIBC_2.3 |
| `librt.so.1` | 3 | GLIBC_2.2.5 |

The sniper runtime ships glibc 2.31, which covers every version. 312 further imports
carry no version and are provided by no bundled module:

- 287 belong to the runtime's non-glibc libraries above (GLib, HarfBuzz, libva,
  Xlib, Expat, FriBidi, bzip2, zlib, VDPAU, libdrm);
- 4 are `dlopen`, `dlsym`, `dlclose`, and `dlerror`, imported by FFmpeg without
  version information, so the glibc layer must also answer unversioned lookups;
- 21 are weak references the code tests before use: `getentropy`, the memory
  protection key functions `pkey_*` (glibc 2.27 and later), transactional memory and
  profiling hooks, and C++ TLS wrapper functions. They may stay unresolved.

The C++ runtime is satisfied by the sniper runtime downloaded in step A5: the two
modules that need `libstdc++.so.6` require at most GLIBCXX_3.4.20 and CXXABI_1.3.5
(the runtime provides GLIBCXX_3.4.28 and CXXABI_1.3.12), and `libgcc_s` is needed
up to GCC_4.0.0 (the runtime provides GCC_7.0.0).

## Q-4: interposition

No bundled module defines `malloc`, `free`, `calloc`, `realloc`, or any other C
allocator entry point: allocation goes to glibc. 198 imports of system symbols are
also defined by a bundled module, and would bind to it in a global lookup scope:

- 182 are C++ runtime symbols, 173 of them from `libanimationsystem.so`, which
  exports a statically linked C++ runtime, including `operator new` and
  `operator delete`;
- 16 are C++ ABI entry points (`__cxa_throw`, `__cxa_guard_acquire`,
  `__gxx_personality_v0`, and others) and `stderr`.

Which definition wins depends on the load order and the `dlopen` flags of the
engine, which only C4 can observe. The loader must implement the glibc lookup
scope exactly, not a simplification of it. (INFERRED)

## Q-5: TLS models

| Evidence | Count |
| --- | --- |
| Modules with a TLS segment | 40 |
| Modules flagged `DF_STATIC_TLS` | 0 |
| `R_X86_64_DTPMOD64` relocations | 126 |
| `R_X86_64_DTPOFF64` relocations | 36 |
| `R_X86_64_TPOFF64` (initial-exec in a shared object) | 0 |
| `R_X86_64_TLSDESC` | 0 |
| Modules importing `__tls_get_addr` | 40 |

Every shared object reaches its TLS through `__tls_get_addr` (global- and
local-dynamic). No module uses TLS descriptors or initial-exec, so the loader needs
static TLS only for the executable's own 16-byte block, and dynamic TLS for the rest.

## B5: the `fs` rewrite on the real modules

| Measure | Value |
| --- | --- |
| Code bytes (executable sections) | 254,118,761 |
| Covered by `.eh_frame` FDEs | 48.8% |
| FDEs read, compared with `.eh_frame_hdr` counts | equal in all 61 modules |
| `fs` accesses inside FDE ranges | 17,734 |
| of which `fs:0x28` (stack guard) | 17,733 |
| of which `fs:0` (thread pointer) | 1, in `cs2` |
| `fs:0x30`, constant offset, register | 0 |
| Prefix byte located for a same-length rewrite | 17,734 of 17,734 |
| Decoder failures inside FDE ranges | 0 |
| Modules with no `fs` access outside FDE ranges | 52 of 61 |

The `fs` counts inside FDE ranges equal `objdump`'s counts of `%fs:` operands in
every module checked.

Outside the FDE ranges a linear sweep of the executable sections finds 533 `fs`
prefixes in 9 modules. They are of two kinds:

- **10 real thread-pointer reads in `libtier0.so`** (`mov fs:0, reg` and
  `cmp reg, fs:0`), the same 10 that `objdump` finds, in functions with ordinary
  compiled prologues. They lie in internal functions that are neither exported nor
  described by an FDE. `libtier0.so` is the engine's base library, and only 16% of
  its code has unwind information.
- **523 decodes of data embedded in `.text`**: 403 string instructions through
  `fs:rsi` (`movs`, `cmps`, `outs`), which compilers do not emit, and 120 memory
  operands with displacements that read as ASCII text. The same sequences recur in
  `libclient.so`, `libfilesystem_stdio.so`, `libpanorama_text_pango.so`,
  `libvideo.so`, and `libsteamnetworkingsockets.so`, which points at one statically
  linked component with tables inside its code. (INFERRED from the forms and the
  repetition; not proven code-free)

Two measurement errors were found and fixed on the way, each in its own commit. Some
modules are linked with one executable segment that also maps `.dynstr`, `.rodata`,
and `.eh_frame`, and the first sweep decoded their strings (`d` is `0x64`, the `fs`
prefix). The scan now bounds code by the executable sections. String instructions
had been counted as `fs:0` because they have no ModRM memory operand, and now have
their own kind.

## What this means for the design

- **The canary is the whole story inside FDE ranges.** Of 17,734 accesses there,
  17,733 read the stack guard and one is the executable's thread pointer. Every one
  can be rewritten mechanically.
- **The coverage condition of the [TLS decision](TLS_UNDER_ROSETTA_DECISION.md) is
  not met by FDEs alone.** Valve's modules carry unwind tables mainly for functions
  that need them, so coverage ranges from 2.6% to 98% and is 48.8% overall. Under the
  decision as written, `libtier0.so` would refuse to load: its 10 thread-pointer
  reads are real code outside any FDE.
- **The next measurement is a code-discovery rule.** The decision allows decoding
  outside FDE ranges when an independent rule proves the bytes are code. The rule
  to build and measure is recursive descent: from FDE ranges, exported functions,
  the entry point, and initializer arrays, follow direct calls and jumps. It must
  reach the 10 sites in `libtier0.so` and must not reach the 523 data decodes.
- **The loader's TLS work is smaller than planned.** No initial-exec, no TLSDESC,
  no static TLS in shared objects: `__tls_get_addr` and one static block.
- **The libc surface is sized.** 710 glibc symbols, the C++ runtime from sniper, and
  the runtime's other libraries for FFmpeg and Pango. This is the input to the libc
  bake-off of roadmap step C3.
