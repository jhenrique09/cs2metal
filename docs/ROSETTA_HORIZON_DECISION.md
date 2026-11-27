# Decision: stay on Rosetta for now

## Status

Accepted on 27 November 2026 by the project owner. Revisit at the M9 gate, or
earlier if Apple publishes the scope of the Rosetta subset it keeps after macOS 27.

## Context

The host is an x86-64 process that runs the game's x86-64 code under Rosetta 2.
Apple states that macOS 27 is the last release in which Rosetta is a general-purpose
tool, and that a subset stays available afterwards, including for some older,
unmaintained games ([research](FEASIBILITY_RESEARCH_2026-11-26.md), §4). macOS 28 is
expected in the fall of 2027. The scope of the retained subset is UNKNOWN.

The [route and effort review](ROUTE_AND_EFFORT_REVIEW_2026-11-27.md) estimates 6 to 18
months of focused work to the first real frame, so the first frame may well arrive
after Rosetta stops being general-purpose. It asked for an explicit choice now
between two product expectations:

| Option | Meaning |
| --- | --- |
| A. **A research prototype on Rosetta** | Accept the Rosetta horizon. Build everything on Rosetta 2 and the GS mechanism measured by the [probe](ROSETTA_SEGMENT_PROBE_2026-11-26.md). |
| B. A durable host | Budget an owned x86-64 to arm64 translator as a separate major project, before or alongside the host. |

## Decision

Option A. A native arm64 port, which needs an instruction translator of its own, is
a much larger project than the host itself. The project continues on Rosetta, as
a research prototype, and accepts that its useful life may end with general-purpose
Rosetta.

## Consequences

- The development machine stays on macOS 27, where Rosetta is fully supported, and
  the project tests every macOS 28 beta as soon as it is available to learn whether
  the retained subset covers this process.
- No work goes into an x86-64 to arm64 translator now. It stays in
  [FUTURE_WORK.md](FUTURE_WORK.md) as the exit from Rosetta.
- Two rules keep that exit open at no extra cost:
  - The Linux system call layer (the glibc ABI layer before the
    [libc route decision](LIBC_ROUTE_DECISION.md)), the substitution table, and the
    Steam bridge are written so
    that they do not depend on executing under Rosetta. Only the loader's code
    transforms and the GS bridge are Rosetta-specific.
  - Rosetta-specific mechanisms stay behind one module boundary in the loader.
- The private and undocumented facts the host relies on (the
  `thread_fast_set_cthread_self` call and the location of the pthread TSD block) are
  discovered or checked at run time, never hard-coded, so that a macOS update fails
  loudly instead of corrupting state.
- Success is measured as research results: each gate of the
  [roadmap](ROADMAP.md) passed or failed with evidence. Reaching a playable state is
  not promised.
