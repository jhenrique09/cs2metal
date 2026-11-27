# Agent guidelines

This file defines the working rules for automated contributors. Human contributors
should read [CONTRIBUTING.md](CONTRIBUTING.md), which covers the same invariants.

The documentation lives in `docs/`, in this repository. Read
`docs/ROADMAP.md`, `docs/ARCHITECTURE.md`, `docs/IMPLEMENTATION_PLAN.md`, and
`docs/ANTI_CHEAT_AND_INTEGRITY_POLICY.md` before changing behavior.

## Language

- Write all source code, identifiers, comments, configuration keys, filenames,
  commit messages, and documentation in English.
- Conversation with the project owner may be in Portuguese.

## Current scope

- Milestone M0 is done: repository foundations, the read-only ELF parser, the
  installation inventory, and platform probes under `spikes/`.
- `docs/ROADMAP.md` is the ordered path from here to the game's first frame. Work in
  its step order, and run the probes it puts first before building what they test.
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
  in the ignored `reports/` directory. Documentation may cite counts, the names of
  public interfaces (glibc, SDL, Vulkan, the public Steamworks API), and the launch
  options and configuration keys needed to describe behavior, not Valve's own export
  lists or quoted strings.
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

- **Every change gets its own commit.** Each feature implemented, each fix, each
  refactor, and each documentation change is committed as soon as it is complete and
  `tools/check.sh` passes. Do not leave finished work uncommitted, and do not batch
  unrelated changes into one commit.
- A feature and the tests and documentation that belong to it go in the same commit.
  A fix found while working on something else is committed separately, first.
- English, imperative subject line under 72 characters, a body that says why.
- Keep the tree green at every commit: `tools/check.sh` passes before each one.
- Update `docs/IMPLEMENTATION_PLAN.md` and `docs/ROADMAP.md` when a milestone item or
  a roadmap step changes state, with the validation evidence, in the commit that
  changes it.
- Do not push, and do not rewrite published history, unless the owner asks.
