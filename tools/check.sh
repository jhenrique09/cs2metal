#!/usr/bin/env bash
# Runs every check a commit must pass. There are no Git hooks: run this before
# every commit. Without arguments it checks the whole working tree; --staged makes
# the repository guard read the Git index instead.
set -euo pipefail

cd "$(git rev-parse --show-toplevel)"
mode="${1:---all}"

echo "==> repository guard tests"
python3 -m unittest discover -s tools/tests -q

echo "==> repository guard ($mode)"
python3 tools/repo_guard.py "$mode"

echo "==> documentation checker tests"
python3 -m unittest discover -s docs/tests -q

echo "==> documentation checker"
python3 docs/tools/check_docs.py

if [[ -f Cargo.toml ]]; then
    echo "==> cargo fmt"
    cargo fmt --all -- --check

    echo "==> cargo clippy"
    cargo clippy --workspace --all-targets --all-features --locked -- -D warnings

    echo "==> cargo test"
    cargo test --workspace --all-features --locked

    # Guest code runs only in an x86-64 process, under Rosetta 2 on Apple silicon:
    # the loader is also linted and tested there.
    echo "==> cargo clippy and test, x86_64-apple-darwin (Rosetta 2)"
    cargo clippy -p cs2metal-loader --target x86_64-apple-darwin --all-targets --locked -- -D warnings
    cargo test -p cs2metal-loader --target x86_64-apple-darwin --locked
    cargo clippy -p cs2metal-linux -p cs2metal --target x86_64-apple-darwin --all-targets --locked -- -D warnings
    cargo test -p cs2metal-linux --target x86_64-apple-darwin --locked
    cargo test -p cs2metal --target x86_64-apple-darwin --locked --test exec
fi

echo "==> all checks passed"
