# Route and effort review, 27 November 2026

## Status

Review snapshot of 27 November 2026, requested by the project owner. This document
is non-normative: it evaluates the accepted Linux-build route and the current
roadmap, but none of its recommendations becomes a decision until the relevant
decision record and roadmap are changed. It is based on the repository state, its
validation results, and a local reading of Native VIctory for macOS. As a dated
review, it is not rewritten after the fact; newer findings get a new document.

## Executive assessment

**The route makes sense as a systems-research project. It does not make sense as a
pragmatic way to play Counter-Strike 2 on a Mac.**

If the objective is to play the game, the existing Windows-build route through
Wine and a Direct3D translation layer has a much better effort-to-result ratio. If
the objective is to build an owned host without Wine or a virtual machine, the
Linux build remains the best of the options considered in the
[Linux build decision](LINUX_BUILD_BASELINE_DECISION.md).

The recommendation is to continue through a short set of feasibility gates, but
not to commit months to the full loader and custom glibc layer yet. The largest
risk is the CPU and operating-system runtime, not graphics. The first frame would
be an important research result, not evidence that playable performance or
stability is near.

## Evidence and current maturity

On 27 November 2026:

- The repository contains about 4,700 lines across source, tests, probes, and
  tools. The implemented product code covers ELF parsing, installation inventory,
  and the integrity baseline.
- `tools/check.sh` passes: repository and documentation guards, formatting,
  Clippy, and 60 tests. This is **CONFIRMED** by running it on the reviewed tree.
- The Linux CS2 installation has not been downloaded. Only the two C++ runtime
  libraries from the public Steam Linux Runtime have been parsed from this
  ecosystem. No CS2 instruction has executed under the host.
- The loader, glibc layer, SDL and Vulkan substitutions, Steam bridge, and
  `cs2metal run` do not exist.
- The GS swap mechanism is measured, but the proposed `fs` to `gs` instruction
  rewrite is neither implemented nor measured on real modules.

These facts mean that the green check suite demonstrates a healthy foundation,
not feasibility of the complete route.

[Native VIctory for macOS](https://github.com/LakshanSivakumar/Native-Victory-macOS)
provides useful prior art: its own loader, GS domain switching, threads, signals,
and host calls work under Rosetta, and its README reports rendered boot video and
menus but not gameplay. This is **REPORTED** for project status and **CONFIRMED**
for the mechanisms read in its source. It validates the general host mechanism,
not the cost or outcome of this project.

## Why the chosen route is coherent

- Linux and macOS x86-64 share the System V calling convention and the Itanium
  C++ ABI. This avoids the per-call Windows x64 conversion required by an owned
  Windows host.
- Vulkan and SDL have macOS implementations of the same C APIs. Substituting them
  avoids writing a game-specific Direct3D renderer and shader translator.
- Keeping the Linux C++ runtime as guest code avoids incompatible C++ standard
  library layouts.
- The roadmap puts inexpensive, potentially fatal experiments before most of the
  implementation: the real inventory, an x86-64 Vulkan presentation probe, and a
  Steam client probe.
- Mechanical, build-independent compatibility transforms and a rerun inventory
  are the right maintenance policy for a frequently updated game.

The public Vulkan research is cautiously encouraging. No currently known hard
requirement is missing on paper from both candidate drivers. The lack of
`VK_EXT_graphics_pipeline_library` points to shader-compilation stutter rather
than an obvious boot blocker. Actual x86-64 operation under Rosetta and the
renderer runtime queries remain unmeasured.

## Changes recommended before a large investment

### 1. Make T-001 coverage an early gate

The `fs` to `gs` rewrite is the central correctness risk. The
[TLS decision](TLS_UNDER_ROSETTA_DECISION.md) proposes decoding function ranges
from `.eh_frame`, but unwind metadata does not guarantee coverage of every leaf,
assembly function, or executable region.

Before building the full loader, add a static analyzer that reports, per module:

- The number and kind of `fs` accesses found.
- The proportion of executable bytes covered by frame description entries and by
  any other accepted code-range evidence.
- Decoder failures, ambiguous regions, and unexplained `fs` uses.
- Which instructions would be rewritten and which regions cannot be proven safe.

Start with the owned fixtures and the already downloaded C++ runtime libraries,
then run the analyzer on the real game installation. Detailed derived reports
stay in ignored local directories; tracked documents record only counts and
public ABI names, as the policy requires.

The fixture gate should cover threads, stack protectors, static and dynamic TLS,
C++ exceptions between guest modules, nested guest-host-guest calls, and signals
on both sides of a domain transition. Any exception crossing a host callback must
be explicitly supported and tested where the public API permits it, or explicitly
rejected as unsupported.

### 2. Close the inventory's TLS reporting gap

The roadmap expects step B2 to answer which TLS models the modules use. At the
time of this review, the inventory stores global relocation totals and only
`uses_tls` and `static_tls` booleans per module. It does not classify
initial-exec, local-exec, global-dynamic, local-dynamic, and TLSDESC use per
module.

Extend the inventory before claiming that B2 closes Q-5. The same per-module
relocation data will help size both the loader and the two possible libc routes.

### 3. Compare custom glibc with real glibc plus syscall translation now

The custom glibc ABI layer is likely to be the largest source of code and
behavioral bugs: allocation, pthreads, stdio, locale, signals, unwinding,
`dlopen`, `/proc`, and many subtle edge cases.

The [future-work list](FUTURE_WORK.md) already names the alternative of loading
the runtime's real glibc and translating Linux system calls, but defers it until
the custom layer becomes a bottleneck. That ordering risks waiting until the
largest sunk cost has already been incurred.

Run a time-boxed bake-off before the full glibc milestone. Both routes should run
the same small C and C++ fixtures. Compare the required imports or syscalls,
observed behavioral gaps, bridge complexity, and implementation effort. The
syscall route is not assumed to be easier: `clone`, futex, `epoll`, signals,
`/proc`, and Mach-O substitutions are substantial problems. The purpose is to
choose from measurements rather than intuition.

### 4. Reuse Native VIctory components in M2, not only in future work

The projects share the GPL-3.0 licence, and the local Native VIctory source has
directly relevant implementations of:

- Explicit, nestable GS-domain switches.
- Dynamic discovery of the host pthread TSD block.
- Signal entry and interrupted-domain detection.
- Thread and synchronization primitives.
- Interface thunks and bring-up diagnostics.

Adapt these pieces with the required attribution and copyright notices. In
particular, do not treat `pthread_self() + 0xE0` as a stable ABI: Native VIctory
searches for the TSD block because its offset can vary between macOS versions.

The PE-specific loader and Windows exception model are not drop-in ELF or Itanium
C++ solutions. Reuse the platform mechanisms, not assumptions belonging to the
Windows route.

### 5. Decide the acceptable Rosetta horizon now

The host depends on Rosetta, a private thread call, and undocumented pthread
layout facts. macOS 27 is the last general-purpose Rosetta release, and coverage
by the retained subset is unknown.

Before the largest investment, choose between two explicit product expectations:

- A time-limited research prototype that accepts the Rosetta horizon.
- A durable host that budgets for an owned x86-64 to arm64 translator as a
  separate major project.

Leaving this entirely to M9 risks completing the first host near the end of its
platform's useful life.

### 6. Treat the listed fallbacks as new projects

An arm64 rendering helper, an owned Vulkan-to-Metal implementation, and an owned
instruction translator are not small fallbacks. They cross different process,
memory-sharing, graphics, or compiler boundaries and each needs its own effort
assessment and decision record.

If no x86-64 Vulkan driver can present under Rosetta, the default recommendation
is to stop and reassess, not to proceed implicitly to Vulkan over inter-process
communication. Similarly, a first rendered frame must not be used as the sole
estimate for the work required to reach a stable local bot match.

## Effort assessment

These are orders of magnitude, not commitments. They assume an experienced
systems engineer working close to full time, with selective Native VIctory reuse.
Part-time work is more likely to be measured in years. No real CS2 execution has
yet provided a basis for a narrower estimate.

| Result | Plausible cumulative effort |
| --- | --- |
| Real inventory, driver and Steam probes, and T-001 static analysis | Days to several weeks |
| Loader with owned C and C++ fixtures, threads, TLS, and callbacks | 1 to 3 months |
| First real CS2 frame | 6 to 18 months |
| Stable local bots, audio, and reasonable performance | 1 to 3 years, or possibly never |

The dominant cost is expected to be debugging asynchronous and behavioral
differences, not generating lines of code. Online competitive play is not
included in any estimate and remains outside the current objective.

## Recommended gates

### Gate 0: evidence before the runtime investment

1. Download the Linux build and run the real inventory and baseline.
2. Run F1: create and present through an x86-64 Vulkan driver under Rosetta.
3. Run H2 against the signed-in macOS Steam client.
4. Measure T-001 code coverage on the real modules and complete per-module TLS
   reporting.

### Gate 1: compare the core runtime choices

1. Build the owned C and C++ fixtures.
2. Test the GS bridge, callbacks, signals, threads, and exceptions on them.
3. Run the time-boxed custom-glibc versus real-glibc comparison.
4. Adapt the proven Native VIctory platform mechanisms.

### Gate 2: commit to the implementation route

Only after Gates 0 and 1, revise the architecture, milestones, and effort budget
around the measured route. Stop or reconsider if Vulkan cannot present, if T-001
cannot establish safe coverage on real modules, or if neither libc approach can
run the basic C++ fixtures reliably.

## Recommendation

Continue with Gate 0. Do not build the complete custom glibc layer yet. If the
real inventory, Vulkan presentation, T-001 coverage, and fixture results are
favorable, the Linux route justifies the next investment. If one of its central
mechanisms fails, stopping or changing route is a successful research outcome,
not a failed implementation.

For practical play, keep using the existing Wine-based route. For research, the
cs2metal route is unusual, technically coherent, and worth a bounded feasibility
investment.
