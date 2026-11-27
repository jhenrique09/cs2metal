# Contributing

## Setup

```bash
./install.sh       # once: the game, its runtime, and a first build (docs/GETTING_STARTED.md)
./build.sh         # after a change: build and install to ~/cs2-metal/bin
tools/check.sh     # before every commit
```

The checks need neither the game nor Steam: they run on a fresh clone with rustup
and python3. The tests that run Linux programs under the host are skipped until
their inputs exist: `tools/fixtures.sh` builds the fixtures (in Docker), and
`./build.sh` leaves the guest's C library in `target/guest-glibc/`. The project
uses no Git hooks. Run `tools/check.sh` yourself before
every commit: the repository guard and its tests, the documentation checker and its
tests, the tools' tests, a syntax check of every shell script, `cargo fmt --check`,
`cargo clippy -D warnings`, and the full test suite. Do not commit while any of them
fails.

## Where things are

| Path | Contents |
| --- | --- |
| `apps/cs2metal` | The command-line program |
| `crates/` | The ELF parser, the scans, the loader, and `cs2metal-linux`: the system call layer and the SDL, Vulkan, and Steam bridges |
| `guest/glibc` | The patch and the build of the guest's C library |
| `guest/shims` | The three shim libraries served to the game |
| `tools/` | The check suite, the repository guard, and the helpers of the scripts at the root |
| `docs/` | The architecture, decisions, plan, and dated records; `docs/README.md` is the index |

`docs/ARCHITECTURE.md` explains how they fit together.

## What a change needs

- Tests for every change in behavior, in the same commit.
- English everywhere: code, comments, documentation, commit messages.
- An update under `docs/` when the change affects architecture, a decision, or a
  milestone.
- A measurement, when the change is about what the game does: a dated record under
  `docs/`, with only what the policy allows from a log (counts, system call
  numbers, public interface names).

## What never goes in

- Game files or anything copied or dumped from them.
- Steam credentials, sentry files (`ssfn*`), `loginusers.vdf`, `config.vdf`, or any
  token, key, or password.
- Machine-specific paths such as a real home directory.
- Anything that studies, disables, intercepts, or works around Valve Anti-Cheat, or
  that changes gameplay. `docs/ANTI_CHEAT_AND_INTEGRITY_POLICY.md` has the rules in
  full; they are not up for discussion in a pull request.

The repository guard (`tools/repo_guard.py`) rejects the first three mechanically,
and `.gitignore` keeps the usual locations out of `git status`.

## Commits

Every change gets its own commit: each feature, fix, refactor, or documentation
change is committed once it is complete and `tools/check.sh` passes, together with
its tests. Unrelated changes never share a commit.

## Commit messages

```
Parse GNU symbol versions in the dynamic segment

Imports carry the library that defines their version, which the
inventory needs to attribute glibc symbols without guessing.
```

An imperative subject line under 72 characters, and a body that says why.
