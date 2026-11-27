# Agent guidelines

This file defines the working rules for automated contributors. Human contributors
should read [CONTRIBUTING.md](CONTRIBUTING.md), which covers the same invariants.

The documentation lives in `docs/`, in this repository. Read
`docs/ARCHITECTURE.md`, `docs/IMPLEMENTATION_PLAN.md`, and
`docs/ANTI_CHEAT_AND_INTEGRITY_POLICY.md` before changing behavior.

## Language

- Write all source code, identifiers, comments, configuration keys, filenames,
  commit messages, and documentation in English.
- Conversation with the project owner may be in Portuguese.

## Current scope

- Milestone M0: repository foundations, the read-only ELF parser, the installation
  inventory, and platform probes under `spikes/`.
- The target is the native Linux build of Counter-Strike 2 (`game/bin/linuxsteamrt64`).
  The Windows build is out of scope; a Wine-based route already exists elsewhere.
- Record ideas for later milestones in `docs/FUTURE_WORK.md`; do not add partial
  implementations of them.

## Invariants

- Never write to, rename, or delete anything inside a game installation. Open game
  files read-only. The integrity baseline is the authority on what "unmodified" means.
- Every in-memory transform of guest code or data beyond standard dynamic linking
  must have an entry in the transform registry described in `docs/ARCHITECTURE.md`,
  an accepted decision record, and a load-time counter.
- Never study, disable, intercept, or spoof Valve Anti-Cheat, and never emulate Steam
  authentication or ownership. Steam features go through the owner's own Steam client.
- Never track game files, Steam session material, credentials, or dumps derived from
  Valve binaries (symbol tables, disassembly, string lists). Generated reports belong
  in the ignored `reports/` directory. Documentation may cite counts and the names of
  public ABI functions (glibc, SDL, Vulkan), not Valve's own export lists.
- Use neutral paths in tracked files: `~`, `$HOME`, or `<game_dir>`, never a real home
  directory.
- Parsers must never panic on malformed input; return a typed error.
- `unsafe` is forbidden in the current crates. Crates that need it (the loader) opt
  out explicitly and document every unsafe block's invariant.

## Validation

Every commit must pass `tools/check.sh`. The project uses no Git hooks, so run it
yourself before each commit and do not commit while it fails. A change in behavior
comes with tests in the same commit.

Validation never launches the game, contacts Steam, or downloads game files. Steps
that need the game are owner actions recorded in `docs/IMPLEMENTATION_PLAN.md`.

## Commits

- English, imperative subject line under 72 characters, a body that says why.
- One concern per commit. Keep the tree green at every commit.
- Update `docs/IMPLEMENTATION_PLAN.md` when a milestone item changes state, with the
  validation evidence.
