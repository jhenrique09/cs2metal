# cs2metal documentation

This directory holds the architecture, decision records, implementation plan,
research, and policies of cs2metal, an experimental host that runs the native Linux
build of Counter-Strike 2 on Apple silicon Macs. It is tracked in the same repository
as the code. It began as a separate repository and was merged, with its history, on
26 November 2026.

## Current normative documents

| Document | Purpose |
| --- | --- |
| [Getting started](GETTING_STARTED.md) | The owner's first session in seven steps: install Steam and steamcmd, download the Linux build, the C++ runtime and the Vulkan SDK, then run the inventory. |
| [Roadmap to the first frame](ROADMAP.md) | The ordered steps from an empty Mac to the game's startup screen: what to install, what exists, what to build, which probes come first. |
| [Architecture](ARCHITECTURE.md) | Layers, process model, loader, transform registry, bridge, glibc layer, substitution table. |
| [Implementation plan](IMPLEMENTATION_PLAN.md) | Milestones M0 to M9, their gates and evidence, safety boundary, risks. |
| [Anti-cheat, integrity, and legal policy](ANTI_CHEAT_AND_INTEGRITY_POLICY.md) | Owner's risk acceptance, the rules that never bend, distribution and licensing. |

## Decision records

| Decision | Status |
| --- | --- |
| [The Linux build is the baseline](LINUX_BUILD_BASELINE_DECISION.md) | Accepted, 26 November 2026. |
| [Thread-local storage under Rosetta](TLS_UNDER_ROSETTA_DECISION.md) | Proposed: rewrite `fs` prefixes to `gs` in private mappings and swap GS at the boundary. Accepted at the M2 gate. |
| [Stay on Rosetta for now](ROSETTA_HORIZON_DECISION.md) | Accepted, 27 November 2026: a research prototype on Rosetta; no arm64 translator work; Rosetta-specific code isolated. |
| [Steam bridge design](STEAM_BRIDGE_DESIGN.md) | Accepted, 27 November 2026, after probes H-1 and H-2 passed: keep the game's own Linux `libsteam_api.so` and bridge it to the macOS client library, so only Valve's code handles identity and licences. |

## Research

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
