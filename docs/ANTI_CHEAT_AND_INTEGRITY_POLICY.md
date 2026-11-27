# Anti-cheat, integrity, and legal policy

## Status

Normative. Adopted on 26 November 2026. Not legal advice.

## Risk acceptance

On 26 November 2026 the project owner explicitly accepted the risk of running the
Linux build of Counter-Strike 2 through a compatibility host that Valve does not
support, including the risk to the Steam account used for testing. That acceptance
covers the owner's own testing. It does not extend to anyone else, and the rules
below still apply in full.

## What Valve says

- Valve Anti-Cheat names "modifications to a game's core executable files and
  dynamic link libraries" as cheating (Valve's VAC FAQ, as quoted in the
  cs2-apple-silicon project's research, which records the full source trail).
- Valve has published no policy on compatibility layers for CS2 on macOS. The Linux
  build ships with VAC enabled, so VAC does not depend on Windows itself.
- Nothing in this repository can resolve how VAC treats a custom ELF loader. It is
  **unknown**, and the project does not try to find out by studying VAC.

## Absolute rules

1. **Files on disk are never modified.** Game files are opened read-only. The
   integrity baseline (milestone M1) records the SHA-256 of every file in the
   installation, and the host refuses to start when any file differs.
2. **In-memory transforms are registered.** Mapping, relocating, and binding imports
   are what every dynamic loader does and need no further justification. Any other
   change to guest code or data in memory needs an accepted decision record, an
   entry in the transform registry (see [ARCHITECTURE.md](ARCHITECTURE.md)), and a
   load-time counter that the host logs. A transform must be mechanical and
   build-independent: no per-build offsets and no change to game logic.
3. **No gameplay changes and no advantage.** No memory reading or writing for
   game-state purposes, no overlays beyond Steam's, no macros, no input automation,
   no wallhack-adjacent rendering changes.
4. **VAC is out of scope permanently.** The project never studies, disables,
   intercepts, spoofs, or works around Valve Anti-Cheat.
5. **Steam authentication is Valve's.** The project never handles credentials,
   never emulates ownership, and never replaces Steam authentication. Steam features
   reach the owner's own Steam client.
6. **Offline by default.** Until the owner decides otherwise in a dated decision
   record, test launches use `-insecure` and local bot matches, which keeps them off
   VAC-secured servers.
7. **A secondary account for anything online.** Never test online with an account
   whose loss would matter.

## Distribution

- The repositories contain only the project's own code and documentation. No Valve
  file, no Apple SDK component, and no third-party binary is tracked.
- Nothing derived from Valve's binaries is tracked either: no symbol tables, string
  lists, disassembly, or offsets of Valve code. Documents may state counts and name
  public interfaces (glibc, the C++ runtime, SDL, Vulkan extensions and commands, the
  public Steamworks API), and may name the launch options, environment variables,
  and configuration keys needed to describe behavior. Diagnostics are paraphrased,
  not quoted. A public third-party source is cited by link, never copied in.
- Third-party runtime pieces (a Vulkan driver, the Steam Linux Runtime's C++
  libraries, macOS builds of SDL) are downloaded by the owner from their upstreams.

## Licensing

The project's code and documentation are licensed under the GNU General Public
License, version 3 (`LICENSE` at the repository root; SPDX `GPL-3.0-only`), by owner
decision on 26 November 2026. Code from VIctory and Native VIctory for macOS, both
GPL-3.0, may therefore be reused with attribution and their copyright notices kept.

## Trademarks

Counter-Strike, Steam, and Valve are Valve's trademarks. Apple, Metal, macOS, and
Rosetta are Apple's. They are used only to describe compatibility. The project is not
affiliated with or endorsed by either company.
