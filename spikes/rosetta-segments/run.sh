#!/usr/bin/env bash
# Builds the x86-64 probe and runs every case in its own process under Rosetta 2,
# with a timeout, because some cases hang or abort the process.
set -euo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
root="$(git -C "$here" rev-parse --show-toplevel)"
out="$root/target/spikes/rosetta-segments"
mkdir -p "$out"

clang -arch x86_64 -O1 -mfsgsbase -mavx2 -Wno-deprecated-declarations \
    "$here/probe.c" -o "$out/probe"

timeout_seconds="${PROBE_TIMEOUT:-5}"
echo "machine: $(sysctl -n machdep.cpu.brand_string), macOS $(sw_vers -productVersion) ($(sw_vers -buildVersion))"
printf '%-10s %-8s %s\n' case outcome output
for case in cpuid avx2 rdfsbase wrfsbase fs-read ldt-auto ldt-fixed gs-swap; do
    set +e
    output="$(perl -e 'alarm shift; exec @ARGV' "$timeout_seconds" "$out/probe" "$case" 2>&1)"
    status=$?
    set -e
    case "$status" in
        0) outcome=ok ;;
        142) outcome=hang ;;
        134) outcome=abort ;;
        *) outcome="exit $status" ;;
    esac
    printf '%-10s %-8s %s\n' "$case" "$outcome" "$(echo "$output" | tr '\n' ' ')"
done
