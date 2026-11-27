# Implementation plan

## Status

Current as of 27 November 2026. M0 is complete; M1 waits on an owner action.

Milestone numbers are identifiers, not the order of work. [ROADMAP.md](ROADMAP.md)
gives the order, with the exact commands for the owner actions: the Steam and Vulkan
probes of M7 and M6 come early, and the Steam bridge is expected to be needed before
the first frame.

## Objective

Run the unmodified Linux build of Counter-Strike 2 on an Apple silicon Mac through
the host described in [ARCHITECTURE.md](ARCHITECTURE.md), offline against bots
first. Every milestone ends in a gate with evidence recorded here.

## Safety boundary

Automated validation (`tools/check.sh`) never launches the game, contacts Steam, downloads game files, or
writes outside the repository and the system temporary directory. Steps that need
the game or a Steam sign-in are **owner actions**, marked as such below. The rules in
the [policy](ANTI_CHEAT_AND_INTEGRITY_POLICY.md) apply to every milestone.

## Milestones

### M0: foundations, 26 November 2026

- [x] Code repository with the repository guard, the check suite, and a pre-commit
  hook that runs it on every commit.
- [x] Documentation repository nested at `docs/`, with its checker, its tests, and
  its own pre-commit hook.
- [x] `cs2metal-elf`: a read-only ELF64 parser covering program headers, the dynamic
  segment, GNU symbol versions, relocation tables, `RELR`, the TLS template, and
  initializers, with fixtures built in memory and no game file.
- [x] `cs2metal-inventory` and `cs2metal inventory`: import resolution against the
  bundled modules, host requirements by library and family, interposition, TLS and
  `COPY` relocation reporting, Markdown and JSON reports that never name the root.
- [x] Rosetta segment probe, with results recorded in
  [the probe report](ROSETTA_SEGMENT_PROBE_2026-11-26.md).
- [x] Feasibility research, the Linux build decision, and the proposed TLS decision.

Validation: code repository commits `f2e1956`, `da5e6e7`, `ad94de9`, and `3ec42d4`,
each created through the pre-commit hook. At `3ec42d4`: 11 repository guard tests,
14 parser tests and 1 doctest (every truncation and single-byte corruption of a
fixture fails without a panic), 10 inventory tests, and 3 command-line tests pass,
and clippy reports no warnings. Documentation checker: 9 tests pass.

Amended on 26 November 2026, by owner decision: the documentation repository was
merged into `docs/` of the code repository with its history, and both pre-commit
hooks were removed. `tools/check.sh` now covers code and documentation and is run
explicitly before every commit.

### M1: the Linux build and its baseline inventory

- [ ] **Owner action:** install Steam for macOS and steamcmd, then download the Linux
  build with steamcmd into a directory outside the repository, signing in
  interactively (roadmap steps A1 to A4):
  `steamcmd +@sSteamCmdForcePlatformType linux +force_install_dir <game_dir> +login <account> +app_update 730 validate +quit`.
- [x] `cs2metal-integrity` and `cs2metal baseline record|verify`: SHA-256 of every
  file, written to the ignored `baselines/` directory, verified before any later
  milestone touches the installation. Built on 27 November 2026: 11 library tests
  (SHA-256 test vectors, links recorded without being followed, nothing modified on
  disk, every kind of difference in full and quick mode, `steamapps/` excluded) and a
  command-line test (refusal to overwrite, exit status 4 on a difference).
- [x] **Owner action** done on 27 November 2026: steamcmd installed build 26879581
  of the Linux build on the Mac, 71.1 GB, and `baseline record` covered its 3,131
  files.
- [x] Run `cs2metal inventory <game_dir>` and answer open questions Q-1 to Q-5 of the
  [feasibility research](FEASIBILITY_RESEARCH_2026-11-26.md) in a new dated research
  document that records counts and public ABI names only. Done on 27 November 2026:
  [inventory of the Linux build](LINUX_BUILD_INVENTORY_2026-11-27.md).
- [x] Download the Steam Linux Runtime 3.0 ("sniper") from Valve's public repository
  and extract its C++ runtime (roadmap step A5). Done on 27 November 2026: runtime
  3.0.20261021, checksum verified, both libraries parsed by `cs2metal inspect`.
- [x] Check that libstdc++ and libgcc_s 10.3 satisfy the build's symbol versions.
  Done on 27 November 2026: the build needs at most GLIBCXX_3.4.20, CXXABI_1.3.5, and
  GCC_4.0.0; the runtime provides GLIBCXX_3.4.28, CXXABI_1.3.12, and GCC_7.0.0.
- [x] **Owner action:** install the Vulkan SDK for macOS and record `vulkaninfo` for
  each driver (roadmap step A6). Done on 27 November 2026 with SDK 1.4.363.1, per
  driver and per architecture ([Vulkan driver measurement](VULKAN_DRIVER_MEASUREMENT_2026-11-27.md)).

- [x] `cs2metal tls scan` (roadmap step B5): `fs` accesses by kind, unwind-table
  coverage of executable bytes, uncovered `fs` prefixes, decoder failures, and TLS
  models per module. Built on 27 November 2026 on `.eh_frame` range parsing in
  `cs2metal-elf` and the iced-x86 decoder: 6 library tests on hand-assembled code, a
  command-line test, and agreement with `objdump` on five public runtime libraries.
- [x] Run it on the real Linux build. Done on 27 November 2026, after two fixes
  found by checking against `objdump` (code bounded by executable sections; string
  instructions no longer counted as `fs:0`): every `fs` access inside FDE ranges is
  rewritable, FDEs cover 48.8% of the code, and `libtier0.so` has 10 real
  thread-pointer reads outside them.
- [x] Code discovery beyond the unwind tables (roadmap step B6): recursive descent
  that reaches the 10 `libtier0.so` sites and none of the 523 data decodes. Done on
  27 November 2026 with 7 tests on hand-assembled code: all 10 reached and
  rewritable, no data decode reached, 61 of 61 modules clean
  ([code discovery](CODE_DISCOVERY_2026-11-27.md)).

Gate (Gate 0 of the [roadmap](ROADMAP.md), with probes H-2 and F1): the inventory
and the code scan answer Q-1 to Q-5 and measure T-001's coverage on the real modules,
and the M2 to M3 scope below is revised to match.

### M2: the loader, on the project's own fixtures

- [x] Small Linux C and C++ programs, written in this project, built for
  linux/amd64 in a container by a script, verified by checksum. Done on
  27 November 2026: `fixtures/` and `tools/fixtures.sh`, six fixtures in the pinned
  Steam Runtime 3 SDK image, expected outputs recorded from native runs, and
  rebuilds byte-identical to `fixtures/SHA256SUMS`. Binaries are rebuilt, not
  tracked, so the guard needs no allowlist ([fixtures decision](FIXTURES_DECISION.md)). `cs2metal tls scan`
  agrees with `objdump` on all nine files.
- [ ] `cs2metal-loader`: map, relocate, bind with versions, IFUNC, `COPY`, init order,
  static TLS in variant II, the GS bridge, and T-001 behind a flag.
- [ ] Acceptance of the [TLS decision](TLS_UNDER_ROSETTA_DECISION.md): fixtures that
  use `__thread`, the stack protector, and threads run correctly with the rewrite.

Gate: the fixtures print their expected output, including from several threads, and
the T-001 counters match an independent count of `fs` prefixes in their code.

### M3: the libc route

- [ ] Bake-off on the fixtures between the own glibc ABI layer, the runtime's real
  glibc with system calls translated, and glibc built from source with a host
  backend (roadmap step C3). Choose one from measurements (Gate 1).
- [ ] The symbols the M1 inventory lists for the first boot path, by family, for
  the chosen route.
- [ ] Layout translation for every type that differs, with a test per type.
- [ ] pthread objects with glibc sizes and lazy host objects; neutral locks on
  `__ulock_wait` and `__ulock_wake`, measured against libSystem's.
- [ ] Synthesized `/proc` and `getauxval`.

Gate: C++ fixtures that throw exceptions across modules, use threads, and read files
pass with the guest libstdc++ and libgcc_s.

### M4: engine boot, offline

- [ ] `cs2` starts with the integrity guard passing and reaches the engine's own
  initialization logs, with every unresolved import named.

- [ ] Boot as a dedicated server with the empty render system (roadmap step C5).

Gate: the boot reaches the first SDL or Vulkan call, and the dedicated server loads a
map and keeps running for ten minutes.

### M5: SDL3 substitution

- [ ] Window, keyboard and mouse, game controllers, and audio through a macOS SDL3.

Gate: a window opens and audio plays.

### M6: Vulkan on Metal

- [x] Probe for Q-6: an x86-64 Vulkan driver in a Rosetta process presenting to a
  window. Choose KosmicKrisp or MoltenVK on measurement. Done on 27 November 2026
  with `spikes/vulkan-present`: MoltenVK presents from x86-64 under Rosetta 2 with
  the same frame pacing as native; the SDK's KosmicKrisp is arm64 only. MoltenVK is
  the driver ([Vulkan driver measurement](VULKAN_DRIVER_MEASUREMENT_2026-11-27.md)).
- [x] Static Vulkan scan of the Linux renderer module (roadmap step B4):
  `cs2metal vulkan scan` and `vulkan compare`, 6 library tests and a command-line
  test. Done on 27 November 2026: 68 device extension names, 33 offered by MoltenVK;
  both drivers report every subgroup operation and BC compression.
- [ ] Record the renderer's required features and extensions (Q-7), completing the
  [Vulkan research](VULKAN_REQUIREMENTS_RESEARCH_2026-11-27.md) with measurements.
  The static half is done; what the renderer enables and requires is logged in F2,
  including whether its subgroup check needs the vertex stage.

First gate, the goal of the roadmap: the game presents its startup screen in its own
window. Second gate: the main menu renders.

### M7: Steam

- [x] Probe H-1 of the [Steam bridge design](STEAM_BRIDGE_DESIGN.md): confirmed on
  27 November 2026 ([probe](STEAM_CLIENT_PROBE_2026-11-27.md)).
- [ ] Probes H-2 to H-4 (Q-8) against the owner's macOS Steam client. H-2 passed on
  27 November 2026 with `spikes/steam-client`: served as app 730, online, ownership
  answered ([Steam app probe online](STEAM_APP_PROBE_ONLINE_2026-11-27.md)). H-3 and H-4 remain. H-2 before any
  bridge code.
- [ ] The bridged client module and the interface proxies.

Gate: Steam initialization succeeds for app 730 without any change to Steam
authentication.

### M8: offline playable

Gate: thirty minutes of a local bot match on one map with no crash, launched with
`-insecure`, and the integrity guard passing before and after.

### M9: life after Rosetta

By the [Rosetta horizon decision](ROSETTA_HORIZON_DECISION.md) of 27 November 2026,
the project is a research prototype on Rosetta and no translator work is planned.

- [ ] Test every macOS 28 beta, as soon as one is available, for whether the retained
  Rosetta subset still runs this host.

Gate: a dated revision of the Rosetta horizon decision, backed by those tests.

## Risks

| Id | Risk | Mitigation |
| --- | --- | --- |
| R-1 | Rosetta stops serving this process after macOS 27. | Accepted by the [Rosetta horizon decision](ROSETTA_HORIZON_DECISION.md): the project is a research prototype. macOS 28 betas are tested early; the HLE layers are written so that they do not depend on Rosetta. |
| R-2 | `fs` cannot be emulated; the T-001 rewrite misses or corrupts code. | Decoder-based rewrite over known function ranges, counters, and refusal on unexplained prefixes. |
| R-3 | No x86-64 Vulkan driver works acceptably in a Rosetta process. | The M6 probe comes before any graphics work; MoltenVK as fallback; an own Vulkan-to-Metal path as last resort. |
| R-4 | The renderer needs Vulkan features Metal cannot provide. | Q-7 is measured before investing in the renderer path. |
| R-5 | The Steam bridge hypothesis is wrong. | The M7 probe is isolated; offline boot does not depend on it until M7. |
| R-6 | Valve treats the host as a modification and acts on the account. | Accepted by the owner. Offline by default, secondary account, no VAC study. |
| R-7 | Weekly game updates break the host. | No per-build offsets anywhere; the inventory reruns after every update. |
| R-8 | The GS swap makes host calls too slow. | Neutral functions on hot paths; measure call frequency per import in M4. |
| R-9 | `cs2` is a fixed-address executable inside `__PAGEZERO`. | Retired on 27 November 2026: `cs2` is a PIE (Q-1). |
