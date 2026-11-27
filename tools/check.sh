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
fi

echo "==> all checks passed"
