#!/usr/bin/env bash
# Builds the trap probe for x86-64 and runs each case in its own process under
# Rosetta 2 with a timeout, because the ud2 case hangs.
set -euo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
root="$(git -C "$here" rev-parse --show-toplevel)"
out="$root/target/spikes/syscall-trap"
mkdir -p "$out"

echo "machine: $(sysctl -n machdep.cpu.brand_string), macOS $(sw_vers -productVersion) ($(sw_vers -buildVersion))"
for case in int3 ud2; do
    clang -arch x86_64 -O2 -Wall -DROUNDS=2000 "$here/probe.c" -o "$out/probe"
    set +e
    output="$(perl -e 'alarm shift; exec @ARGV' "${PROBE_TIMEOUT:-20}" "$out/probe" "$case" 2>&1)"
    status=$?
    set -e
    [ "$status" -eq 142 ] && output="hang (no return from the handler within the timeout)"
    echo "== $case: $(echo "$output" | tr '\n' ' ')"
done
