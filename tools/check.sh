#!/usr/bin/env bash
# Runs every check a commit must pass. The pre-commit hook calls this with
# --staged; run it without arguments to check the whole working tree.
set -euo pipefail

cd "$(git rev-parse --show-toplevel)"
mode="${1:---all}"

echo "==> repository guard tests"
python3 -m unittest discover -s tools/tests -q

echo "==> repository guard ($mode)"
python3 tools/repo_guard.py "$mode"

if [[ -f Cargo.toml ]]; then
    echo "==> cargo fmt"
    cargo fmt --all -- --check

    echo "==> cargo clippy"
    cargo clippy --workspace --all-targets --locked -- -D warnings

    echo "==> cargo test"
    cargo test --workspace --locked
fi

echo "==> all checks passed"
