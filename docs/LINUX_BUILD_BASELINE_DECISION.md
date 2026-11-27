# Decision: the Linux build is the baseline

## Status

Accepted on 26 November 2026 by the project owner.

## Context

Counter-Strike 2 ships for Windows and Linux. A Mac can already run the Windows
build through Wine and DXMT ([research §1](FEASIBILITY_RESEARCH_2026-11-26.md)). The
owner wants a host that runs the game natively in the sense Native VIctory for macOS
uses the word: the game's own code in our own process, no emulated operating system,
no virtual machine, and game files untouched on disk.

## Options

| Option | Summary | Why not, or why |
| --- | --- | --- |
| A. Windows build in Wine with DXMT or D3DMetal | What cs2-apple-silicon does | Already solved elsewhere; depends on layers the project would not own. Not this project. |
| B. Windows build with our own Win32 layer | Native VIctory's approach, applied to a PC game | Every call crosses Win64 to System V; the Win32 and Direct3D 11 surface is large; Direct3D 11 still needs a translation to Metal. |
| C. **Linux build with our own glibc layer** | Map the ELF modules, implement the glibc ABI, hand SDL and Vulkan to macOS builds of the same libraries | **Chosen.** |
| D. Linux build in a virtual machine | An arm64 Linux guest with Rosetta for Linux, or an x86-64 emulator | Not native: a second kernel, and GPU access through a paravirtualized device. |

## Decision

Option C. The host is an x86-64 macOS process that loads the Linux ELF modules
unmodified on disk and provides the glibc ABI, the substitution table for SDL,
Vulkan and Steam, and the bridge between Linux and macOS thread conventions.

## Rationale

- **One calling convention.** Linux and macOS on x86-64 share the System V AMD64
  calling convention and the Itanium C++ ABI. Where Option B converts every call,
  Option C calls host functions directly once data layouts agree.
- **A smaller, documented surface.** The glibc ABI is specified, versioned symbol by
  symbol, and has independent implementations (musl, bionic) to learn from. Win32 is
  larger and documented by behavior.
- **The graphics API maps directly.** Vulkan has conformant implementations on Metal.
  The game's Vulkan calls can reach a Vulkan-on-Metal driver without a Direct3D
  translation layer.
- **SDL is portable by construction.** If the build uses SDL3 for windows, input, and
  audio, as reports suggest, its C API is the same on macOS, so a macOS build of SDL3
  can answer those calls.

## Consequences

- The thread-pointer convention differs: Linux code uses `fs`, which Rosetta does not
  support. This is settled separately in the [TLS decision](TLS_UNDER_ROSETTA_DECISION.md).
- glibc data layouts must be translated at the boundary for every type that differs.
- The guest needs a Linux C++ runtime (libstdc++ and libgcc_s), loaded as guest code,
  because libc++ is not layout-compatible.
- The host depends on Rosetta for as long as the guest is x86-64 code. Milestone M9
  answers what happens after macOS 27.
- Every finding in this route is new: no other project runs this build on macOS, so
  each step is planned as an experiment with an explicit gate.
