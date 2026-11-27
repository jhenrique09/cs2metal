# Contributing

## Setup

```bash
tools/check.sh
```

The project uses no Git hooks. Run `tools/check.sh` yourself before every commit: the
repository guard and its tests, the documentation checker and its tests,
`cargo fmt --check`, `cargo clippy -D warnings`, and the full test suite. Do not
commit while any of them fails.

## What a change needs

- Tests for every change in behavior, in the same commit.
- English everywhere: code, comments, documentation, commit messages.
- An update under `docs/` when the change affects architecture, a decision, or a
  milestone.

## What never goes in

- Game files or anything copied or dumped from them.
- Steam credentials, sentry files (`ssfn*`), `loginusers.vdf`, `config.vdf`, or any
  token, key, or password.
- Machine-specific paths such as a real home directory.

The repository guard (`tools/repo_guard.py`) rejects these mechanically, and
`.gitignore` keeps the usual locations out of `git status`.

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
