# Implementation plan

## Status

Current as of 26 November 2026. M0 is complete; M1 waits on an owner action.

## Objective

Run the unmodified Linux build of Counter-Strike 2 on an Apple silicon Mac through
the host described in [ARCHITECTURE.md](ARCHITECTURE.md), offline against bots
first. Every milestone ends in a gate with evidence recorded here.

## Safety boundary

Automated validation (`tools/check.sh`) never launches the game, contacts Steam, downloads game files, or
writes outside the repositories and the system temporary directory. Steps that need
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

- [ ] **Owner action:** download the Linux build with steamcmd into a directory
  outside both repositories, signing in interactively:
  `steamcmd +@sSteamCmdForcePlatformType linux +force_install_dir <game_dir> +login <account> +app_update 730 validate +quit`.
- [ ] `cs2metal-integrity` and `cs2metal baseline record|verify`: SHA-256 of every
  file, written to the ignored `baselines/` directory, verified before any later
  milestone touches the installation.
- [ ] Run `cs2metal inventory <game_dir>` and answer open questions Q-1 to Q-5 of the
  [feasibility research](FEASIBILITY_RESEARCH_2026-11-26.md) in a new dated research
  document that records counts and public ABI names only.
- [ ] **Owner action:** obtain the Steam Linux Runtime 3.0 ("sniper") and run the
  inventory on its C++ runtime, to see which libstdc++ and libgcc_s versions the
  build's symbol versions require.

Gate: the inventory answers Q-1 to Q-5, and the M2 to M3 scope below is revised to
match the real import list.

### M2: the loader, on the project's own fixtures

- [ ] Small Linux C and C++ programs, written in this project, built for
  linux/amd64 in a container by a script, verified by checksum. Tracking these
  binaries needs an allowlist in the repository guard and a decision on whether
  generated fixtures are tracked or rebuilt.
- [ ] `cs2metal-loader`: map, relocate, bind with versions, IFUNC, `COPY`, init order,
  static TLS in variant II, the GS bridge, and T-001 behind a flag.
- [ ] Acceptance of the [TLS decision](TLS_UNDER_ROSETTA_DECISION.md): fixtures that
  use `__thread`, the stack protector, and threads run correctly with the rewrite.

Gate: the fixtures print their expected output, including from several threads, and
the T-001 counters match an independent count of `fs` prefixes in their code.

### M3: the glibc ABI layer

- [ ] The symbols the M1 inventory lists for the first boot path, by family.
- [ ] Layout translation for every type that differs, with a test per type.
- [ ] pthread objects with glibc sizes and lazy host objects; neutral locks on
  `__ulock_wait` and `__ulock_wake`, measured against libSystem's.
- [ ] Synthesized `/proc` and `getauxval`.

Gate: C++ fixtures that throw exceptions across modules, use threads, and read files
pass with the guest libstdc++ and libgcc_s.

### M4: engine boot, offline

- [ ] `cs2` starts with the integrity guard passing and reaches the engine's own
  initialization logs, with every unresolved import named.

Gate: the boot reaches the first SDL or Vulkan call.

### M5: SDL3 substitution

- [ ] Window, keyboard and mouse, game controllers, and audio through a macOS SDL3.

Gate: a window opens and audio plays.

### M6: Vulkan on Metal

- [ ] Probe for Q-6: an x86-64 Vulkan driver in a Rosetta process presenting to a
  window. Choose KosmicKrisp or MoltenVK on measurement.
- [ ] Record the renderer's required features and extensions (Q-7).

Gate: the main menu renders.

### M7: Steam

- [ ] Probes H-1 to H-4 of the [Steam bridge design](STEAM_BRIDGE_DESIGN.md) (Q-8)
  against the owner's macOS Steam client, the first two before any bridge code.
- [ ] The bridged client module and the interface proxies.

Gate: Steam initialization succeeds for app 730 without any change to Steam
authentication.

### M8: offline playable

Gate: thirty minutes of a local bot match on one map with no crash, launched with
`-insecure`, and the integrity guard passing before and after.

### M9: life after Rosetta

Gate, before fall 2027: a dated decision between an own x86-64 to arm64 translator
and Apple's retained Rosetta subset, backed by measurements.

## Risks

| Id | Risk | Mitigation |
| --- | --- | --- |
| R-1 | Rosetta stops serving this process after macOS 27. | M9 decides early; the HLE layers are written so that they do not depend on x86-64 hosting. |
| R-2 | `fs` cannot be emulated; the T-001 rewrite misses or corrupts code. | Decoder-based rewrite over known function ranges, counters, and refusal on unexplained prefixes. |
| R-3 | No x86-64 Vulkan driver works acceptably in a Rosetta process. | The M6 probe comes before any graphics work; MoltenVK as fallback; an own Vulkan-to-Metal path as last resort. |
| R-4 | The renderer needs Vulkan features Metal cannot provide. | Q-7 is measured before investing in the renderer path. |
| R-5 | The Steam bridge hypothesis is wrong. | The M7 probe is isolated; offline boot does not depend on it until M7. |
| R-6 | Valve treats the host as a modification and acts on the account. | Accepted by the owner. Offline by default, secondary account, no VAC study. |
| R-7 | Weekly game updates break the host. | No per-build offsets anywhere; the inventory reruns after every update. |
| R-8 | The GS swap makes host calls too slow. | Neutral functions on hot paths; measure call frequency per import in M4. |
| R-9 | `cs2` is a fixed-address executable inside `__PAGEZERO`. | Link the host with a small page-zero (Q-1 answers whether this is needed). |
