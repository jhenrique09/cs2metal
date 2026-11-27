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
| Rosetta facts: no `fs` base, GS swap works | Measured ([probe](ROSETTA_SEGMENT_PROBE_2026-11-26.md)) |
| Vulkan needs against both drivers, from public data | Researched ([report](VULKAN_REQUIREMENTS_RESEARCH_2026-11-27.md)) |
| Steam bridge | Designed, unproven ([design](STEAM_BRIDGE_DESIGN.md)) |
| The Linux build on this Mac | **Not downloaded** |
| Steam for macOS, steamcmd, Vulkan SDK on this Mac | **Not installed** |
| Loader, glibc layer, bridge, `cs2metal run` | **Not built** |

The game does not start today. Phases A and B can be done today; phases C to F are
the engineering.

## The path at a glance

```
A  Put the pieces on the Mac        owner, today          no code needed
B  Know the build                   tools exist (B1-B2)   answers Q-1 to Q-5
   ├─ D1  Steam probes              needs A2              answers H-1, H-2
   └─ F1  Vulkan driver probe       needs A6              answers Q-6
C  A host that runs Linux code      the bulk of the work  loader, glibc layer
D  Steam bridge                     after C               SteamAPI_Init succeeds
E  A window                         after C               SDL3 on macOS
F  Vulkan device and first frame    after E               ← the goal of this roadmap
```

The three questions most able to stop the project (does Steam on macOS serve app
730, does an x86-64 Vulkan driver work under Rosetta, what kind of executable is
`cs2`) are answered by B2, D1, and F1, before most of the code is written. Do those
first.

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
curl -fLO "$B/$F" && curl -fsSLO "$B/SHA256SUMS"
grep -F "$F" SHA256SUMS | shasum -a 256 -c -
tar -xzf "$F" --include '*/x86_64-linux-gnu/libstdc++.so.6*' \
              --include '*/x86_64-linux-gnu/libgcc_s.so.1'
```

Only the two libraries are extracted. That is all the host loads from the runtime,
and it avoids unpacking a Linux tree onto a case-insensitive volume. The runtime's
glibc is never loaded (cs2metal provides that ABI), and its SDL and Vulkan loader are
replaced by macOS libraries.

Done when: `sniper/files/lib/x86_64-linux-gnu/libstdc++.so.6` resolves to a file.

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

### B3. Integrity baseline (to build, S)

`cs2metal baseline record <game_dir>` writes the SHA-256 of every file to the ignored
`baselines/` directory; `cs2metal baseline verify` compares. Every later step that
opens the installation verifies first. Run `record` only right after steamcmd's
`validate`.

### B4. Static Vulkan scan (to build, S)

`cs2metal vulkan scan <game_dir>` lists the Vulkan extension and command names inside
the Linux renderer module, and `cs2metal vulkan compare` crosses them with the
`vulkaninfo` reports of A6. It replaces the Windows-derived list of the Vulkan
research with the real one.

## Phase C: a host that runs Linux code

This is the bulk of the work, and none of it needs the game until C4.

### C1. Fixtures (to build, M)

Small Linux C and C++ programs written in this project, built for linux/amd64 in a
container by a script: hello world, threads with `__thread` variables, the stack
protector, C++ exceptions across shared objects, `dlopen`, IFUNC. They are the
loader's test suite.

### C2. Loader, bridge, and the `fs` rewrite (to build, L)

`cs2metal-loader`: map, relocate, bind with symbol versions, run initializers, static
TLS, `dlopen`; the GS bridge with neutral and host thunks; transform T-001 behind a
flag. Gate: every fixture prints its expected output, and the
[TLS decision](TLS_UNDER_ROSETTA_DECISION.md) is accepted or revised.

### C3. The glibc ABI layer, first slice (to build, XL)

`cs2metal-glibc`: the symbols the B2 inventory lists, ordered by what the fixtures
and then the engine's boot path call first. Gate: the C++ fixtures pass with the
guest libstdc++ and libgcc_s from A5.

### C4. `cs2metal run` and the first engine boot (to build, M, then iterative)

The command this roadmap is building toward:

```bash
cargo build --release --target x86_64-apple-darwin -p cs2metal
target/x86_64-apple-darwin/release/cs2metal run \
  --game-dir ~/cs2metal-kit/cs2-linux \
  --runtime-dir ~/cs2metal-kit/sniper \
  -- -insecure -novid -windowed -width 1280 -height 720
```

It does not exist yet. When it does, it verifies the baseline, sets the guest
environment, loads `cs2`, and logs to the ignored `runs/` directory: every module
loaded, every file the guest opens, and every unresolved import when first called.
The paths can also come from an ignored `cs2metal.local.toml`.

From here the work is a loop: run, read which import or behavior stopped the boot,
implement it, commit, run again. Gate: the engine's own log shows its Steam
initialization attempt and its first SDL or Vulkan call.

## Phase D: Steam

Design and rules: [STEAM_BRIDGE_DESIGN.md](STEAM_BRIDGE_DESIGN.md). In short, the
game's own Linux `libsteam_api.so` runs unmodified, and only the client library it
loads is bridged to the `steamclient.dylib` of the macOS Steam installed in A2.
Nothing about identity, ownership, or sessions is emulated.

### D1. Probes that need no guest code (to build, S; possible right after A2)

- H-1: `lipo -info` on the `steamclient.dylib` inside the Steam installation shows an
  x86-64 slice.
- H-2: a small native x86-64 program under `spikes/` declares app 730, connects to
  the running client through that library, and asks two read-only questions: is the
  user signed in, and does the account hold app 730.

If H-2 fails, Steam on macOS refuses the app and the design must change before any
bridge code is written. That is why this comes early.

### D2. The bridge (to build, L; after C2)

The `~/.steam` lookups answered from real macOS state, the bridged client module,
the interface proxies with generic domain-switch thunks. Gate: the engine logs a
successful Steam initialization with the signed-in account, launched with
`-insecure`.

Whether the engine can reach its window without Steam is hypothesis H-5; the C4 boot
log answers it. If it can, D2 moves after phase F.

## Phase E: a window

### E1. SDL3 substitution (to build, M; after C4)

Bind the guest's SDL3 imports to an x86-64 build of SDL3 for macOS, through host
thunks, with callbacks entering the guest domain. Events must be pumped on the
process's main thread, as macOS requires. Gate: the game's window opens and receives
keyboard and mouse input.

## Phase F: Vulkan and the first frame

### F1. A Vulkan driver inside a Rosetta process (to build, S; possible right after A6)

A native x86-64 probe under `spikes/` creates an instance and a device with each
driver, opens a window through SDL3, and presents a cleared swapchain image. It
answers Q-6: KosmicKrisp if it can be built for or loaded into an x86-64 process,
otherwise MoltenVK, which ships a universal library. Measure frame time and shader
translation time for both when both work.

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
milestones M5 to M8 of the implementation plan. Life after Rosetta is milestone M9.

## Critical path and parallel work

- **Owner, today:** A1 to A6, then B1 and B2. Nothing else is blocked on anything.
- **Build, in parallel with the downloads:** C1 and C2 need no game file.
- **Probes, as soon as A2 and A6 are done:** D1 and F1. They are small, they are
  independent of the loader, and each can invalidate a design.
- **Critical path:** B2 → C2 → C3 → C4 → (D2) → E1 → F2 → F3.

## What can stop this roadmap

| Finding | Step that reveals it | Then |
| --- | --- | --- |
| steamcmd will not deliver the Linux depot to a Mac | A4 | Steam console `download_depot`, depot by depot |
| `cs2` is a fixed-address executable inside the first 4 GB | B2 | Link the host with a small `__PAGEZERO`; known technique |
| Steam on macOS refuses app 730 | D1 | No Steam for the guest; boot as far as the engine goes without it |
| No x86-64 Vulkan driver works under Rosetta | F1 | An arm64 helper process for rendering, or the own-translator route of milestone M9 |
| The `fs` rewrite cannot be made safe on real modules | C2, C4 | The own-translator route |
| The renderer refuses to start without a capability Metal lacks | F2 | Recorded as a hard limit; nothing is patched |
