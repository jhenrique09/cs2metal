# cs2metal documentation

This directory holds the architecture, decision records, implementation plan,
research, and policies of cs2metal, a host that runs the native Linux build of
Counter-Strike 2 on Apple silicon Macs. It is tracked in the same repository as the
code. It began as a separate repository and was merged, with its history, on
26 November 2026.

## Where to start

| If you want to | Read |
| --- | --- |
| Play | [Getting started](GETTING_STARTED.md), then the risk in the [policy](ANTI_CHEAT_AND_INTEGRITY_POLICY.md) |
| Know what works today | [The client, playable offline](CLIENT_PLAYABLE_2026-11-27.md) and [online play](ONLINE_PLAY_2026-11-27.md) |
| Understand how it works | [Architecture](ARCHITECTURE.md), then the three bridges: [SDL](SDL_BRIDGE.md), [Vulkan](VULKAN_BRIDGE.md), [Steam](STEAM_BRIDGE_DESIGN.md) |
| Know why it is built this way | The [decision records](#decision-records) |
| Contribute | `CONTRIBUTING.md` and `AGENTS.md` at the repository root, then the [implementation plan](IMPLEMENTATION_PLAN.md) |

## Current documents

These describe the project as it is and change with it.

| Document | Purpose |
| --- | --- |
| [Getting started](GETTING_STARTED.md) | Setting up, starting, updating, and removing the game with `install.sh`, `build.sh`, `run.sh`, and `create-shortcut.sh`; where everything is kept; what to do when something goes wrong. |
| [Architecture](ARCHITECTURE.md) | Layers, process model, dynamic linking, the transform registry, the bridge, the system call layer, the installation overlay, the substitution table, diagnostics. |
| [The SDL bridge](SDL_BRIDGE.md) | Window, keyboard, mouse, and sound: how the game's SDL calls reach SDL 3 for macOS, and what differs between the two. |
| [The Vulkan bridge](VULKAN_BRIDGE.md) | How the renderer's Vulkan calls reach MoltenVK, what the bridge adjusts, and the pacing trace. |
| [Roadmap to the first frame](ROADMAP.md) | The ordered steps from an empty Mac to the game's startup screen, as they were taken: what was installed, built, and probed, in which order. The goal is met; kept as the record of the path. |
| [Implementation plan](IMPLEMENTATION_PLAN.md) | Milestones M0 to M9, their gates and evidence, safety boundary, risks; what is next. |
| [Anti-cheat, integrity, and legal policy](ANTI_CHEAT_AND_INTEGRITY_POLICY.md) | The risk and who accepts it, the rules that never bend, distribution and licensing. |

## Decision records

| Decision | Status |
| --- | --- |
| [The Linux build is the baseline](LINUX_BUILD_BASELINE_DECISION.md) | Accepted, 26 November 2026. |
| [Thread-local storage under Rosetta](TLS_UNDER_ROSETTA_DECISION.md) | Accepted, 27 November 2026: rewrite `fs` prefixes to `gs` in private mappings and swap GS at the boundary; applied at `mmap` under the libc route. |
| [Stay on Rosetta for now](ROSETTA_HORIZON_DECISION.md) | Accepted, 27 November 2026: a research prototype on Rosetta; no arm64 translator work; Rosetta-specific code isolated. |
| [How the guest gets its C library](LIBC_ROUTE_DECISION.md) | Accepted, 27 November 2026: glibc 2.31 rebuilt with one system call slot, with its own ld.so (route C); the host is a Linux system call layer. |
| [Linux fixtures are rebuilt, not tracked](FIXTURES_DECISION.md) | Proposed, 27 November 2026, in effect for C1: track sources, expected outputs, and checksums; build in the pinned SDK image. |
| [Steam bridge design](STEAM_BRIDGE_DESIGN.md) | Accepted, 27 November 2026, after probes H-1 and H-2 passed: keep the game's own Linux `libsteam_api.so` and bridge it to the macOS client library, so only Valve's code handles identity and licences. |
| [An opt-in online launch](ONLINE_LAUNCH_DECISION.md) | Accepted, 27 November 2026: `run.sh` accepts `-online`, which leaves `-insecure` out; offline stays the default, and automated boots never use it. |
| [The shortcut may be added to Steam's library](STEAM_SHORTCUT_DECISION.md) | Accepted, 27 November 2026: `create-shortcut.sh --steam` adds one entry to Steam's list of non-Steam games, only when asked, only while Steam is not running, keeping a copy of the file. |

## Research and dated records

In the order they were written. A dated record is not rewritten: a later one
corrects it and links back.

| Document | Summary |
| --- | --- |
| [Feasibility research, 26 November 2026](FEASIBILITY_RESEARCH_2026-11-26.md) | Prior art, what is known about the Linux build, platform facts, open questions Q-1 to Q-9. |
| [Rosetta segment probe, 26 November 2026](ROSETTA_SEGMENT_PROBE_2026-11-26.md) | No `fs` base is possible under Rosetta; the GS swap works at 520 ns per pair on an M1 Max. |
| [Code scan of the Steam Linux Runtime, 27 November 2026](CODE_SCAN_RUNTIME_2026-11-27.md) | `cs2metal tls scan` matches `objdump` exactly on five runtime libraries; no `fs` access lies outside the unwind tables, even in glibc's assembly. |
| [Steam client probe, 27 November 2026](STEAM_CLIENT_PROBE_2026-11-27.md) | The macOS client library has an x86-64 slice and exports the expected boundary: hypothesis H-1 holds. |
| [Vulkan requirements research, 27 November 2026](VULKAN_REQUIREMENTS_RESEARCH_2026-11-27.md) | The 71 extensions the CS2 renderer names, against MoltenVK and KosmicKrisp; what the game configuration requires; what is still unmeasured. |
| [The CS2 Linux build, inventoried and scanned, 27 November 2026](LINUX_BUILD_INVENTORY_2026-11-27.md) | Q-1 to Q-5 answered: a PIE launcher, 710 glibc symbols, dynamic TLS only. 17,733 of 17,734 `fs` accesses inside FDEs are the canary, but FDEs cover 48.8% of the code and `libtier0.so` has 10 real thread-pointer reads outside them. |
| [Code discovery beyond the unwind tables, 27 November 2026](CODE_DISCOVERY_2026-11-27.md) | Recursive descent proves 91.9% of the build's code, reaches libtier0's 10 thread-pointer reads and no data; all 61 modules can be rewritten mechanically. |
| [Vulkan drivers measured on the Mac, 27 November 2026](VULKAN_DRIVER_MEASUREMENT_2026-11-27.md) | MoltenVK presents from an x86-64 process under Rosetta 2 (Q-6); the SDK's KosmicKrisp is arm64 only. Of the Linux renderer's 68 device extension names MoltenVK offers 33; both drivers report every subgroup operation and BC compression. |
| [Steam app probe, 27 November 2026](STEAM_APP_PROBE_2026-11-27.md) | Partial H-2: the macOS client accepts an x86-64 process as app 730 and answers that the account holds it; it was offline, so the server connection is measured again. |
| [Steam app probe online, 27 November 2026](STEAM_APP_PROBE_ONLINE_2026-11-27.md) | H-2 holds: the connected macOS client serves app 730 to an x86-64 process and answers for the account's licence. Gate 0 passes. |
| [The libc bake-off, 27 November 2026](LIBC_BAKEOFF_2026-11-27.md) | Route B ruled out (a trapped system call costs about 34 µs under Rosetta); route C sends 970 of 975 glibc system call sites through one slot with 20 source lines; route A means 682 functions, about 430 crossing or stateful. |
| [The first engine boot under cs2metal, 27 November 2026](FIRST_ENGINE_BOOT_2026-11-27.md) | As a dedicated server the engine loads 25 modules and SDL3, finishes its Source 2 initialization, starts loading de_dust2, and stops at SteamGameServer_Init: the C4 gate is met, and the Steam bridge is next. |
| [The client's first frames, 27 November 2026](CLIENT_FIRST_FRAMES_2026-11-27.md) | The renderer presents the main menu and a match once `gettimeofday`, the audio callback's contract, and null descriptor sets are right; the roadmap's gate is met. |
| [The dedicated server's ten-minute run, 27 November 2026](DEDICATED_SERVER_RUN_2026-11-27.md) | With sockets emulated, the dedicated server loads a map and runs for ten minutes; roadmap step C5. |
| [The first boots through the Steam bridge, 27 November 2026](STEAM_BRIDGE_BOOT_2026-11-27.md) | The game's own `libsteam_api.so`, unmodified, initializes a game server through the macOS client library; roadmap step D2. |
| [The first client boots, 27 November 2026](FIRST_CLIENT_BOOT_2026-11-27.md) | The client opens its window and creates its Vulkan device; its reading of the first frame is corrected by the next record. |
| [What the client's window shows, 27 November 2026](CLIENT_RENDERING_2026-11-27.md) | Which window shows what, and whether the renderer presents: the correction to the first client boots. |
| [The client, playable offline, 27 November 2026](CLIENT_PLAYABLE_2026-11-27.md) | The owner plays a practice match with keyboard, mouse, and sound at 50 to 80 frames per second; what was fixed for input, the pacing measured, and what stays open. |
| [Online play, as the owner reports it, 27 November 2026](ONLINE_PLAY_2026-11-27.md) | The owner plays casual and competitive matches online, with no VAC ban so far. Reported, not measured; at the player's own risk. |
| [Route and effort review, 27 November 2026](ROUTE_AND_EFFORT_REVIEW_2026-11-27.md) | Owner-requested assessment of the chosen route, its technical gates, alternative libc strategies, Native VIctory reuse, and realistic effort. |

## Backlog

| Document | Purpose |
| --- | --- |
| [Future work](FUTURE_WORK.md) | Ideas that need a decision record before they enter the plan. |

## Documentation policy

English is the only language for tracked documentation. Code identifiers, commands,
and paths stay in English too.

- Evidence is tagged **CONFIRMED** (measured here or read from a primary source),
  **REPORTED** (secondary source, cited), **INFERRED** (reasoned from confirmed facts),
  or **UNKNOWN**. A claim moves up only with new evidence, recorded with its date.
- Research and probe results are dated in their file name (`NAME_YYYY-MM-DD.md`) and
  are not rewritten later. Newer findings get a new dated document that links back.
- Every document other than this index opens with a `## Status` section.
- Dates are absolute ("26 November 2026"), never relative.
- Paths are neutral: `~`, `$HOME`, `<game_dir>`. Never a real home directory.
- Nothing derived from Valve's binaries is recorded beyond counts, public interface
  names, and the option and configuration key names needed to describe behavior, as
  the [policy](ANTI_CHEAT_AND_INTEGRITY_POLICY.md) explains.
- No credential, token, Steam session file, or machine-specific report belongs here.

## Checks

```bash
tools/check.sh
```

Run from the repository root, the check suite includes the documentation checker
(`docs/tools/check_docs.py`) and its tests. The project uses no Git hooks, so run it
before every commit. The checker rejects non-English prose, broken relative
links, home paths, likely secrets, missing status sections, and invalid dates in file
names.
