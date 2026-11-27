# Contributing

## Setup

```bash
git config core.hooksPath .githooks
tools/check.sh
```

The hook runs `tools/check.sh --staged` before every commit: the repository guard
and its tests, `cargo fmt --check`, `cargo clippy -D warnings`, and the full test
suite. A commit that fails any of them is not created. Do not use `--no-verify`.

The hook checks the working tree for Rust and the Git index for the repository
guard. Stage complete changes rather than partial hunks so both see the same code.

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

## Commit messages

```
Parse GNU symbol versions in the dynamic segment

Imports carry the library that defines their version, which the
inventory needs to attribute glibc symbols without guessing.
```
