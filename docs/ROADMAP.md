# Roadmap to the first frame

## Status

Current as of 27 November 2026. This is the ordered, step-by-step path from an empty
Mac to the first frame the game presents in its own window (its startup screen). It
says who does each step, what exists today, and what must still be built.
[IMPLEMENTATION_PLAN.md](IMPLEMENTATION_PLAN.md) stays the ledger of milestones and
evidence; this document is the order of work. Update a step's state in the commit
that changes it.

## Where things stand

| Piece | State |
| --- | --- |
| Repository, check suite, policy, licence | Done |
| ELF parser, `cs2metal inspect`, `cs2metal inventory` | Done |
| Integrity baseline, `cs2metal baseline record` and `verify` | Done |
| Code scan, `cs2metal tls scan` | Done; validated on public runtime libraries |
| Rosetta facts: no `fs` base, GS swap works | Measured ([probe](ROSETTA_SEGMENT_PROBE_2026-11-26.md)) |
| Vulkan needs against both drivers, from public data | Researched ([report](VULKAN_REQUIREMENTS_RESEARCH_2026-11-27.md)) |
| Steam bridge | Designed, unproven ([design](STEAM_BRIDGE_DESIGN.md)) |
| The Linux build on this Mac | **Not downloaded** |
| Steam for macOS, steamcmd, the sniper C++ runtime on this Mac | Installed (A2, A3, A5); Steam not yet signed in |
| Steam client library usable from an x86-64 process (H-1) | Confirmed ([probe](STEAM_CLIENT_PROBE_2026-11-27.md)) |
| Vulkan SDK on this Mac | **Not installed** |
| Loader, glibc layer, bridge, `cs2metal run` | **Not built** |

The game does not start today. Phases A and B can be done today; phases C to F are
the engineering.

## The path at a glance

```
A  Put the pieces on the Mac        owner, today          no code needed
B  Know the build                   tools exist (B1-B3)   answers Q-1 to Q-5, T-001 coverage
   ├─ D1  Steam probes              needs A2              answers H-1, H-2
   └─ F1  Vulkan driver probe       needs A6              answers Q-6
   ═══ Gate 0: evidence before the runtime investment ═══
C  A host that runs Linux code      the bulk of the work  fixtures, loader, libc route
   ═══ Gate 1: the libc route, chosen by measurement ═══
   └─ C5  Dedicated server boot     after C4              no window, no Vulkan
D  Steam bridge                     after C               SteamAPI_Init succeeds
E  A window                         after C               SDL3 on macOS
F  Vulkan device and first frame    after E               ← the goal of this roadmap
```

The questions most able to stop the project (does Steam on macOS serve app 730, does
an x86-64 Vulkan driver work under Rosetta, what kind of executable is `cs2`, can the
`fs` rewrite be proven safe on the real modules) are answered by B2, B5, D1, and F1,
before most of the code is written. Do those first.

The two gates come from the
[route and effort review](ROUTE_AND_EFFORT_REVIEW_2026-11-27.md):

- **Gate 0** passes when B2, B3, B5, B6, D1 (H-2), and F1 have run on the real build and
  none of them invalidates the design. Until then nothing beyond fixtures and the
  loader core is built.
  State: **passed on 27 November 2026.** B2, B3, B5, B6, D1 (H-2), and F1 have run
  on build 26879581 and the owner's machine, and none invalidates the design. Phase
  C may start; Gate 1 comes next.
- **Gate 1** passes when the libc route of C3 is chosen from measurements on the
  fixtures. The architecture and the effort budget are revised around it before the
  route is built out.

Stop or reassess, rather than reaching for a fallback, if no Vulkan driver can
present under Rosetta (F1), if the `fs` rewrite cannot be proven safe on the real
modules (B5), or if no libc route runs the C++ fixtures reliably (C3). Each fallback
named in this document is a project of its own and needs its own decision record.

Sizes are relative: S (hours), M (days), L (weeks), XL (more).

## Phase A: put the pieces on the Mac

Everything lives in one directory outside this repository and outside the Steam
library, so that neither Git nor the macOS Steam client ever touches the game files:

```
~/cs2metal-kit/
  steamcmd/     Valve's command-line downloader, with its own sign-in cache
  cs2-linux/    the Linux build of CS2; cs2metal only ever reads it
  sniper/       the C++ runtime from Steam Linux Runtime 3.0
```

### A1. Prerequisites (owner, S)

An Apple silicon Mac on macOS 26 or newer (KosmicKrisp needs Metal 4), about 80 GB
free, and:

```bash
softwareupdate --install-rosetta --agree-to-license   # no-op when already installed
xcode-select --install                                # no-op when already installed
rustup target add x86_64-apple-darwin                 # the host is an x86-64 process
```

State on the development machine: macOS 27.2.1, Rosetta present, rustup present, the
x86-64 target **not yet added**.

### A2. Steam for macOS (owner, S)

Install Valve's client from the official disk image and sign in. The game will talk
to this client at run time (phase D); it is not used for downloading.

```bash
curl -fL -o ~/Downloads/steam.dmg https://cdn.fastly.steamstatic.com/client/installer/steam.dmg
open ~/Downloads/steam.dmg          # drag Steam to Applications, start it, sign in
```

- Use the account the [policy](ANTI_CHEAT_AND_INTEGRITY_POLICY.md) allows: a
  secondary one for anything online.
- **Do not install CS2 from this client.** macOS Steam queues the platform-neutral
  content plus an empty macOS stub and never the Linux binaries. (REPORTED by
  cs2-apple-silicon, which measured it.) If it starts, cancel it.

Done when: Steam runs and shows the signed-in account.

### A3. steamcmd (owner, S)

Valve's own downloader, as a tarball from Valve's CDN: no package manager, nothing
installed system-wide. It is an x86-64 program, so it runs under Rosetta, and it
updates itself on first start. (CONFIRMED: 2.5 MB tarball, x86-64 Mach-O.)

```bash
mkdir -p ~/cs2metal-kit/steamcmd && cd ~/cs2metal-kit/steamcmd
curl -sSfL https://steamcdn-a.akamaihd.net/client/installer/steamcmd_osx.tar.gz | tar -xz
```

`brew install --cask steamcmd` is the alternative if you prefer Homebrew to manage it.

### A4. The Linux build of CS2 (owner, S to run, hours to download)

steamcmd downloads for the platform it is told, not the one it runs on:

```bash
cd ~/cs2metal-kit/steamcmd
./steamcmd.sh +@sSteamCmdForcePlatformType linux \
  +force_install_dir ~/cs2metal-kit/cs2-linux \
  +login <account> \
  +app_update 730 validate +quit
```

- You type the password and the Steam Guard code into steamcmd itself. No project
  tool sees them. steamcmd keeps its session cache inside `~/cs2metal-kit/steamcmd/`.
- Expect about 60 GB to download and 71 GB on disk: the shared content depots plus
  the Linux depot 2347773. (REPORTED sizes, August 2026.)
- The platform override is documented by Valve for `windows`, `macos`, and `linux`,
  and cs2-apple-silicon used it for `windows`. Nobody is on record using `linux` from
  a Mac, so **this step is itself the first experiment.** (REPORTED mechanism,
  UNKNOWN outcome.)
- CS2 is free, but the account must hold its licence. If steamcmd reports that there
  is no subscription, request the free licence with `+app_license_request 730` before
  `+app_update`, or start the game's installation once from any Steam client.
- To update the game later, run the same command again, then redo B2 and B3.

State: done on 27 November 2026. The `linux` override works from macOS: build
26879581, 71.1 GB, `cs2` is an x86-64 ELF
([inventory](LINUX_BUILD_INVENTORY_2026-11-27.md)).

Done when:

```bash
file ~/cs2metal-kit/cs2-linux/game/bin/linuxsteamrt64/cs2    # ELF 64-bit LSB ... x86-64
```

If the override does not deliver the Linux depot, the fallback is the Steam client's
console (`steam://open/console`, then `download_depot 730 2347773` and the shared
depots), which downloads single depots into `steamapps/content/` and leaves the
assembly to you. Third-party downloaders that take your credentials are not an
option under the policy.

### A5. The C++ runtime from Steam Linux Runtime 3.0 (owner, S)

The game is built against the "sniper" runtime, whose C++ runtime must run as guest
code. Valve publishes the runtime openly, with checksums, so no sign-in is needed.
(CONFIRMED: version 3.0.20261021 in `latest-public-stable`, 305 MB; it carries
glibc 2.31, libstdc++ and libgcc_s 10.3, SDL 3.4, and the Vulkan loader 1.4.309.)

```bash
mkdir -p ~/cs2metal-kit/sniper && cd ~/cs2metal-kit/sniper
B=https://repo.steampowered.com/steamrt3/images/latest-public-stable
F='com.valvesoftware.SteamRuntime.Platform-amd64,i386-sniper-runtime.tar.gz'
curl -fL --http1.1 --retry 5 --retry-all-errors -C - -O "$B/$F" \
  && curl -fsSL --retry 5 -O "$B/SHA256SUMS" \
  && grep -F "$F" SHA256SUMS | shasum -a 256 -c - \
  && tar -xzf "$F" --include '*/x86_64-linux-gnu/libstdc++.so.6*' \
                   --include '*/x86_64-linux-gnu/libgcc_s.so.1'
```
The commands are chained and the download resumes (`-C -`) and retries, because the
first attempt on the development machine was cut by an HTTP/2 stream error at 59%
and the unchained commands that followed ran against a truncated file.


Only the two libraries are extracted. That is all the host loads from the runtime,
and it avoids unpacking a Linux tree onto a case-insensitive volume. The runtime's
glibc is never loaded (cs2metal provides that ABI), and its SDL and Vulkan loader are
replaced by macOS libraries.

Done when: `sniper/files/lib/x86_64-linux-gnu/libstdc++.so.6` resolves to a file.

State: **done on the development machine on 27 November 2026.** Runtime
3.0.20261021, checksum verified. `cs2metal inspect` reads both libraries: libstdc++
6.0.28 imports 128 versioned symbols from `libc.so.6` and 12 from `libgcc_s.so.1`,
and libgcc_s imports 12 from `libc.so.6`. These are the first real ELF files the
parser has read. (CONFIRMED)

### A6. The Vulkan SDK for macOS (owner, S)

Install LunarG's Vulkan SDK from <https://vulkan.lunarg.com/sdk/home>. It provides
`vulkaninfo`, MoltenVK, and, per LunarG, KosmicKrisp. Then record what each driver
reports on this Mac:

```bash
mkdir -p reports
VK_DRIVER_FILES=<MoltenVK ICD manifest from the SDK> \
  vulkaninfo --json -o reports/vulkaninfo-moltenvk.json
VK_DRIVER_FILES=<KosmicKrisp ICD manifest from the SDK> \
  vulkaninfo --json -o reports/vulkaninfo-kosmickrisp.json
```

`VK_DRIVER_FILES` is the Vulkan loader's variable for choosing a driver; the SDK
documents where its manifests are installed. `reports/` is ignored by Git. These files turn the "UNKNOWN" cells of the
[Vulkan research](VULKAN_REQUIREMENTS_RESEARCH_2026-11-27.md) into measurements:
subgroup operation bits, BC texture compression, and every limit.

State: done on 27 November 2026 with SDK 1.4.363.1. `arch` drops `DYLD_*`
variables, so pass them with `arch -e`; the SDK's KosmicKrisp has no x86-64 slice
([Vulkan driver measurement](VULKAN_DRIVER_MEASUREMENT_2026-11-27.md)).

## Phase B: know the build

### B1. Build cs2metal and run the checks (owner, S; exists today)

```bash
cd <cs2metal checkout>
tools/check.sh
cargo build --release
```

### B2. Inventory the Linux build (owner, S; exists today)

```bash
cargo run --release -p cs2metal -- inventory ~/cs2metal-kit/cs2-linux \
  --markdown reports/inventory.md --json reports/inventory.json
cargo run --release -p cs2metal -- inspect ~/cs2metal-kit/cs2-linux/game/bin/linuxsteamrt64/cs2
cargo run --release -p cs2metal -- inventory ~/cs2metal-kit/sniper \
  --markdown reports/sniper.md
```

This answers Q-1 to Q-5 of the
[feasibility research](FEASIBILITY_RESEARCH_2026-11-26.md): the executable type, the
bundled and the missing libraries, the list of glibc symbols to implement, allocator
interposition, and the TLS models in use. Record the answers, as counts and public
names only, in a new dated research document, and resize phase C to match.

State: run on 27 November 2026; Q-1 to Q-5 are answered in the
[inventory of the Linux build](LINUX_BUILD_INVENTORY_2026-11-27.md). `cs2` is a PIE,
the build imports 710 glibc symbols, no bundled module replaces the C allocator, and
every shared object uses dynamic TLS through `__tls_get_addr`.

### B3. Integrity baseline (owner, S; exists today)

```bash
cargo run --release -p cs2metal -- baseline record ~/cs2metal-kit/cs2-linux
cargo run --release -p cs2metal -- baseline verify ~/cs2metal-kit/cs2-linux          # full
cargo run --release -p cs2metal -- baseline verify ~/cs2metal-kit/cs2-linux --quick  # fast
```

`record` writes the SHA-256, size, and modification time of every file, and the
target of every symbolic link, to the ignored `baselines/cs2-linux.json`. It refuses
to replace an existing baseline without `--force`. `verify` exits with status 4 and
lists the differences when anything is missing, added, modified, or relinked. The
`steamapps/` directory is left out: it is steamcmd's own bookkeeping, rewritten on
every update.

Run `record` only right after steamcmd's `validate`, and again after every update.
Every later step that opens the installation verifies first. `--quick` hashes again
only the files whose size or modification time changed.

State: built on 27 November 2026, and recorded the same day for build 26879581:
3,131 files, 71.1 GB, in 11 seconds on all cores; the first quick verify found no
difference.

### B5. Code scan: `fs` accesses, unwind coverage, TLS models (exists today)

```bash
cargo run --release -p cs2metal -- tls scan ~/cs2metal-kit/cs2-linux \
  --markdown reports/tls-scan.md --json reports/tls-scan.json
```

`cs2metal tls scan <file-or-dir>` decodes, read-only, every function that the
`.eh_frame` unwind tables describe in each executable segment, and reports per
module:

- every `fs`-prefixed instruction, classified as a thread-pointer load (`fs:0`), a
  stack-guard read (`fs:0x28`), a pointer-guard read (`fs:0x30`), a local-exec access
  (`fs` with a constant offset), or an initial-exec access (`fs` with a register);
- how many code bytes the unwind tables cover, and the `fs` prefixes, by kind, found
  in the code they do not cover, which T-001 could not rewrite safely. Code is the
  executable sections when the module has a section table, because some linkers map
  read-only data in the executable segment;
- decoder failures inside covered ranges;
- the TLS models in use, from the dynamic relocations (global- and local-dynamic,
  TLS descriptors, initial-exec in shared objects) and from the code (local-exec and
  link-time initial-exec, which leave no dynamic relocation).

That is the per-module answer to Q-5 that the inventory alone cannot give, and the
coverage measurement the [TLS decision](TLS_UNDER_ROSETTA_DECISION.md) needs before
it can be accepted. Detailed reports stay in `reports/`; tracked documents record
counts only. It exits with status 4 when any module's rewrite is not clean.

State: built on 27 November 2026 and validated against `objdump` on public Steam
Linux Runtime libraries: the `fs` counts match exactly, and glibc's hand-written
assembly has no `fs` access outside the unwind ranges
([code scan of the runtime](CODE_SCAN_RUNTIME_2026-11-27.md)).

Run on the game on 27 November 2026
([inventory and scan](LINUX_BUILD_INVENTORY_2026-11-27.md)). Inside FDE ranges all
17,734 `fs` accesses are rewritable, and 17,733 of them read the stack guard. The
FDEs cover only 48.8% of the code, though, and `libtier0.so` has 10 real
thread-pointer reads outside them. On its own the scan did not pass; B6 closed the
gap the same day.

### B6. Code discovery beyond the unwind tables (exists today)

The [TLS decision](TLS_UNDER_ROSETTA_DECISION.md) lets T-001 decode outside FDE
ranges only where an independent rule proves the bytes are code. Add that rule to
`cs2metal tls scan`: recursive descent from the FDE ranges, the exported functions,
the entry point, and the initializer arrays, following direct calls and jumps inside
the executable sections. Report per module the code it reaches beyond the FDEs and
the `fs` accesses found there.

Done when the rule reaches the 10 thread-pointer reads in `libtier0.so`, reaches
none of the 523 data decodes the linear sweep finds, and `objdump` agrees with every
count. If it cannot separate the two, B5's stop condition applies.

State: done on 27 November 2026
([code discovery](CODE_DISCOVERY_2026-11-27.md)). Discovery proves 43.0% of the code
beyond the FDEs (91.9% in total) with no conflict on trusted paths. It reaches all 10
`libtier0.so` reads and none of the 523 data decodes, of which 514 lie in data the
code loads by `lea` and 9 are `fs` string instructions. All 61 modules are clean.
The `lea` rule is a measured heuristic, so the loader recounts at load time.

### B4. Static Vulkan scan (exists today)

`cs2metal vulkan scan <game_dir>` lists the Vulkan extension and command names inside
the Linux renderer module, and `cs2metal vulkan compare` crosses them with the
`vulkaninfo` reports of A6. It replaces the Windows-derived list of the Vulkan
research with the real one.

```bash
cargo run --release -p cs2metal -- vulkan compare ~/cs2metal-kit/cs2-linux \
  reports/vulkaninfo-*.json --markdown reports/vulkan-compare.md
```

State: built and run on 27 November 2026. The Linux module names 68 device
extensions; MoltenVK offers 33 and KosmicKrisp 35, and both report every subgroup
operation and BC compression ([Vulkan driver measurement](VULKAN_DRIVER_MEASUREMENT_2026-11-27.md)).

## Phase C: a host that runs Linux code

This is the bulk of the work, and none of it needs the game until C4.

### C1. Fixtures (exists today)

Small Linux C and C++ programs written in this project, built for linux/amd64 in a
container by a script: hello world, threads with `__thread` variables, the stack
protector, C++ exceptions across shared objects, `dlopen`, IFUNC. They are the
loader's test suite.

```bash
CS2METAL_BUILD_HOST=<ssh destination> tools/fixtures.sh
```

State: built on 27 November 2026. Six fixtures, nine ELF files, built in the
Steam Runtime 3 SDK image pinned by digest on an x86-64 Debian virtual machine.
They run natively there with their expected output, and a rebuild is byte-identical
to the tracked checksums. Binaries are rebuilt, not tracked, by the [fixtures decision](FIXTURES_DECISION.md).

### C2. Loader, bridge, and the `fs` rewrite (to build, L)

`cs2metal-loader`: map, relocate, bind with symbol versions, run initializers, static
TLS, `dlopen`; the GS bridge with neutral and host thunks; transform T-001 behind a
flag. Gate: every fixture prints its expected output, and the
[TLS decision](TLS_UNDER_ROSETTA_DECISION.md) is accepted or revised.

Slices, each with a test that runs guest code under Rosetta 2 in `tools/check.sh`:

1. One module: map, relocate (`RELATIVE`, `GLOB_DAT`, `JUMP_SLOT`, `64`,
   `IRELATIVE`), protect with RELRO, run initializers and finalizers, call exports.
   **Done, 27 November 2026:** the `dlopen` fixture's plugin runs its constructor
   and answers `plugin_answer(20) = 41` inside the x86-64 host.
2. A symbol scope across modules: versions, weak symbols, interposition, `$ORIGIN`.
   **Done, 27 November 2026:** the `scope` fixture's three libraries load breadth
   first through `$ORIGIN` and answer exactly as on Linux, including a weak
   definition overridden and a call interposed inside its own module.
3. TLS and the GS bridge: per-thread blocks, `__tls_get_addr`, T-001 behind a flag,
   host and neutral thunks, with the `tls` and `stack_guard` fixtures.
   **Done, 27 November 2026, on the `tlsprobe` fixture,** which needs no C library:
   T-001 rewrites its 13 `fs` accesses; global-dynamic, local-dynamic,
   initial-exec, and `.tbss` variables give the Linux values in three host threads
   and the main one; the canary and the thread pointer go through `gs`; and a call
   out to a host function crosses the bridge and back. The `tls` and `stack_guard`
   executables need the C library of slice 4.
4. The C library the `hello` fixture needs, which meets the C3 bake-off.

### C3. The libc route, chosen by measurement, then its first slice (to build, M then XL)

Before the large investment, a time-boxed bake-off runs the same C and C++ fixtures
through three ways of giving the guest a C library:

| Route | Summary | What to measure |
| --- | --- | --- |
| Own glibc ABI layer | An own C library in Rust: the symbols the inventory lists, with layout translation | Symbols needed, layout translations, host crossings per fixture |
| The runtime's real glibc, system calls translated | Load glibc 2.31 from the sniper runtime unchanged and translate Linux system calls | Each `syscall` instruction is 2 bytes and cannot become a 5-byte call in place, so this needs a second transform or a trap per system call; measure that cost under Rosetta |
| glibc built from source with a host backend | Compile glibc 2.31 (LGPL) with system calls that call into the host, so no binary patching of glibc | Build effort, how much of glibc's system call layer is centralized, crossings per fixture |

The choice is not only about code size. With a real glibc, `malloc`, `memcpy`, and the
fast paths of pthreads stay inside the guest and never pay the GS swap (about 520 ns
per round trip); only system calls cross. With an own layer, every libc call crosses
unless it is a neutral function. Count crossings per fixture for each route.

Gate (Gate 1): one route chosen with the measurements recorded in a dated research
document, then its first slice built until the C++ fixtures pass with the guest
libstdc++ and libgcc_s from A5.

State: the bake-off ran on 27 November 2026 ([results](LIBC_BAKEOFF_2026-11-27.md)).
Route B is ruled out: a trapped system call costs about 34 µs under Rosetta 2.
Route C sends 970 of glibc's 975 system call sites through one slot with 20 changed
source lines and a two-minute build. The owner chose route C with glibc's own ld.so
the same day ([decision](LIBC_ROUTE_DECISION.md)).

First slice, done on 27 November 2026: `tools/guest-glibc.sh` builds Debian's glibc
2.31 with the slot, and `cs2metal exec` starts its ld.so as a kernel would and
answers its system calls. Six of the eight fixtures, the C++ one included, print
their Linux output under Rosetta 2. The two that use threads followed the same day
with `clone` and `futex`: all eight fixtures pass, which **closes Gate 1**.

```bash
CS2METAL_BUILD_HOST=<ssh destination> tools/guest-glibc.sh
target/x86_64-apple-darwin/debug/cs2metal exec --glibc target/guest-glibc \
  --library-path ~/cs2metal-kit/sniper/files/lib/x86_64-linux-gnu \
  target/fixtures/cxx
```

### C4. `cs2metal run` and the first engine boot (exists today, then iterative)

The command this roadmap is building toward:

```bash
CS2METAL_BUILD_HOST=<ssh destination> tools/guest-glibc.sh
cargo build --release --target x86_64-apple-darwin -p cs2metal
target/x86_64-apple-darwin/release/cs2metal run \
  --game-dir ~/cs2metal-kit/cs2-linux \
  --runtime-dir ~/cs2metal-kit/sniper/files \
  -- -dedicated -insecure +map de_dust2 2> runs/boot.log
```

It verifies the installation against its baseline (quick mode) and refuses to start
on any difference. Then, as the game's own `cs2.sh` does on Linux, it starts
`bin/linuxsteamrt64/cs2` from `game/`, with `bin/linuxsteamrt64` on the library
path, on the runtime's libraries (`files/lib/x86_64-linux-gnu`, extracted from the
A5 tarball). The rebuilt glibc comes first. Unsupported system calls are named once
on standard error, and `CS2METAL_TRACE=1` logs every call. A per-run log directory
and a `cs2metal.local.toml` for the paths can follow.

State: built on 27 November 2026 and tested on a stand-in installation, the `hello`
fixture installed as `cs2`. **Gate met the same day**
([first engine boot](FIRST_ENGINE_BOOT_2026-11-27.md)): as a dedicated server the
engine loads its 25 modules, loads SDL3, finishes its Source 2 initialization,
starts loading `de_dust2`, and stops at `SteamGameServer_Init`, which looks for the
Linux Steam client library. That is step D2.

From here the work is a loop: run, read which import or behavior stopped the boot,
implement it, commit, run again. Gate: the engine's own log shows its Steam
initialization attempt and its first SDL or Vulkan call.

### C5. Boot as a dedicated server first (to build, M; after C4)

The Linux build ships `librendersystemempty.so` beside `librendersystemvulkan.so`
(CONFIRMED from the file names during the download). Source 2 runs its dedicated
server from the same binaries with the empty render system, and Valve's CS2 server
documentation starts it with `-dedicated` (INFERRED for this build; to be confirmed
on the real files). A dedicated server needs no window, no SDL video, and no Vulkan:

```bash
target/x86_64-apple-darwin/release/cs2metal run --game-dir ~/cs2metal-kit/cs2-linux \
  --runtime-dir ~/cs2metal-kit/sniper -- -dedicated -insecure +map de_dust2
```

It exercises the loader, the libc route, threads, the file system, physics, and
networking, so it validates the whole CPU side before phases E and F, and decouples
the boot from the Vulkan probe. Gate: the server loads a map and keeps running for
ten minutes.

State: **gate met on 27 November 2026**
([ten-minute run](DEDICATED_SERVER_RUN_2026-11-27.md)). With sockets emulated, the
server loads `de_dust2`, logs on to Steam anonymously through the bridge, and ran
651 seconds until its timeout, with no fatal error.

## Phase D: Steam

Design and rules: [STEAM_BRIDGE_DESIGN.md](STEAM_BRIDGE_DESIGN.md). In short, the
game's own Linux `libsteam_api.so` runs unmodified, and only the client library it
loads is bridged to the `steamclient.dylib` of the macOS Steam installed in A2.
Nothing about identity, ownership, or sessions is emulated.

### D1. Probes that need no guest code (to build, S; possible right after A2)

- H-1: `lipo -info` on the `steamclient.dylib` inside the Steam installation shows an
  x86-64 slice. **Done, 27 November 2026: confirmed**
  ([probe](STEAM_CLIENT_PROBE_2026-11-27.md)).
- H-2: a small native x86-64 program under `spikes/` declares app 730, connects to
  the running client through that library, and asks two read-only questions: is the
  user signed in, and does the account hold app 730. **Done, 27 November 2026:
  confirmed** with `spikes/steam-client/run.sh`. Offline, the client already accepted
  the process as app 730 ([first run](STEAM_APP_PROBE_2026-11-27.md)); online it is
  also connected to the Steam servers ([Steam app probe online](STEAM_APP_PROBE_ONLINE_2026-11-27.md)).

If H-2 fails, Steam on macOS refuses the app and the design must change before any
bridge code is written. That is why this comes early.

### D2. The bridge (to build, L; after C2)

The `~/.steam` lookups answered from real macOS state, the bridged client module,
the interface proxies with generic domain-switch thunks. Gate: the engine logs a
successful Steam initialization with the signed-in account, launched with
`-insecure`.

State: built on 27 November 2026, the dedicated-server half working
([first boots through the bridge](STEAM_BRIDGE_BOOT_2026-11-27.md)). The shim at
`~/.steam/sdk64/steamclient.so` binds to the macOS client library, client objects
reach the guest as proxies, and the game's own `SteamGameServer_Init` succeeds and
begins its anonymous logon. The client half, with the signed-in account, waits for
the client boot of phase E.

Whether the engine can reach its window without Steam is hypothesis H-5; the C4 boot
log answers it. If it can, D2 moves after phase F.

## Phase E: a window

### E1. SDL3 substitution (to build, M; after C4)

Bind the guest's SDL3 imports to an x86-64 build of SDL3 for macOS, through host
thunks, with callbacks entering the guest domain. Events must be pumped on the
process's main thread, as macOS requires. Gate: the game's window opens and receives
keyboard and mouse input.

State: the bridge is built, 27 November 2026 ([SDL bridge](SDL_BRIDGE.md)): the
game's `libSDL3.so.0` is answered with a shim bound by name to the Vulkan SDK's
universal SDL 3, 1,270 of its 1,302 functions, with guest callbacks through the
guest thunk. The first client boot loads it; the window is next.

## Phase F: Vulkan and the first frame

### F1. A Vulkan driver inside a Rosetta process (exists today)

A native x86-64 probe under `spikes/` creates an instance and a device with each
driver, opens a window through SDL3, and presents a cleared swapchain image. It
answers Q-6: KosmicKrisp if it can be built for or loaded into an x86-64 process,
otherwise MoltenVK, which ships a universal library. Measure frame time and shader
translation time for both when both work.

```bash
spikes/vulkan-present/run.sh
```

State: run on 27 November 2026. MoltenVK presents from x86-64 under Rosetta 2 with
the native build's frame pacing; the SDK's KosmicKrisp loads only in arm64
processes. Q-6 is answered: MoltenVK ([Vulkan driver measurement](VULKAN_DRIVER_MEASUREMENT_2026-11-27.md)).

### F2. Instance and device creation (to build, M; after E1)

Bind the guest's Vulkan loader imports to the macOS loader. Log the instance and
device creation calls as they pass: the extensions and features the renderer asks
for and the ones it refuses to start without. That closes Q-7 and completes the
Vulkan research table with measurements. Gate: the renderer creates its device.

### F3. Swapchain and first present (to build, M to L)

Surface through `SDL_Vulkan_CreateSurface`, which on macOS yields a Metal layer;
swapchain; the first frames. **Gate, and the goal of this roadmap: the game presents
its startup screen in its own window.** Record a screenshot description, the log,
and the driver used in the implementation plan.

## After the first frame

The main menu, a local bot match (`-insecure`), audio, and performance work follow in
milestones M5 to M8 of the implementation plan. The project stays on Rosetta, as a
research prototype, by the [Rosetta horizon decision](ROSETTA_HORIZON_DECISION.md).

## Critical path and parallel work

- **Owner, today:** A1 to A6, then B1, B3 (`baseline record`), and B2. Nothing else
  is blocked on anything.
- **Build, in parallel with the downloads:** B5 runs on the sniper runtime's two
  libraries before the game arrives; C1 and C2 need no game file.
- **Probes, as soon as A2 and A6 are done:** D1 and F1. They are small, they are
  independent of the loader, and each can invalidate a design.
- **Critical path:** B2 → B5 → B6 → Gate 0 → C2 → C3 (Gate 1) → C4 → C5 → (D2) → E1 →
  F2 → F3.

## What can stop this roadmap

| Finding | Step that reveals it | Then |
| --- | --- | --- |
| steamcmd will not deliver the Linux depot to a Mac | A4 | Steam console `download_depot`, depot by depot |
| `cs2` is a fixed-address executable inside the first 4 GB | B2 | Ruled out on 27 November 2026: `cs2` is a PIE |
| Code outside the unwind tables cannot be told from data | B6 | Stop and reassess T-001. Did not happen: separated on 27 November 2026 |
| Steam on macOS refuses app 730 | D1 | No Steam for the guest; boot as far as the engine goes without it. Did not happen: served as app 730, 27 November 2026 |
| No x86-64 Vulkan driver works under Rosetta | F1 | Stop and reassess. An arm64 rendering helper is a new project with its own decision record. Did not happen: MoltenVK works, 27 November 2026 |
| The `fs` rewrite cannot be made safe on real modules | C2, C4 | The own-translator route |
| The renderer refuses to start without a capability Metal lacks | F2 | Recorded as a hard limit; nothing is patched |
