# cs2metal documentation

This directory holds the architecture, decision records, implementation plan,
research, and policies of cs2metal, an experimental host that runs the native Linux
build of Counter-Strike 2 on Apple silicon Macs. It is tracked in the same repository
as the code. It began as a separate repository and was merged, with its history, on
26 November 2026.

## Current normative documents

| Document | Purpose |
| --- | --- |
| [Architecture](ARCHITECTURE.md) | Layers, process model, loader, transform registry, bridge, glibc layer, substitution table. |
| [Implementation plan](IMPLEMENTATION_PLAN.md) | Milestones M0 to M9, their gates and evidence, safety boundary, risks. |
| [Anti-cheat, integrity, and legal policy](ANTI_CHEAT_AND_INTEGRITY_POLICY.md) | Owner's risk acceptance, the rules that never bend, distribution and licensing. |

## Decision records

| Decision | Status |
| --- | --- |
| [The Linux build is the baseline](LINUX_BUILD_BASELINE_DECISION.md) | Accepted, 26 November 2026. |
| [Thread-local storage under Rosetta](TLS_UNDER_ROSETTA_DECISION.md) | Proposed: rewrite `fs` prefixes to `gs` in private mappings and swap GS at the boundary. Accepted at the M2 gate. |

## Research

| Document | Summary |
| --- | --- |
| [Feasibility research, 26 November 2026](FEASIBILITY_RESEARCH_2026-11-26.md) | Prior art, what is known about the Linux build, platform facts, open questions Q-1 to Q-9. |
| [Rosetta segment probe, 26 November 2026](ROSETTA_SEGMENT_PROBE_2026-11-26.md) | No `fs` base is possible under Rosetta; the GS swap works at 520 ns per pair on an M1 Max. |

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
- Nothing derived from Valve's binaries is recorded beyond counts and public ABI
  names, as the [policy](ANTI_CHEAT_AND_INTEGRITY_POLICY.md) explains.
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
