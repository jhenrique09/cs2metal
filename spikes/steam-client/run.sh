#!/usr/bin/env bash
# Builds the H-2 probe for x86_64, the host's architecture under Rosetta 2, and runs
# it once against the running, signed-in Steam client, declared as app 730. While it
# runs, Steam may show the account as playing the game for a few seconds.
set -euo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
root="$(git -C "$here" rev-parse --show-toplevel)"
out="$root/target/spikes/steam-client"
mkdir -p "$out"

steam="$HOME/Library/Application Support/Steam/Steam.AppBundle/Steam/Contents/MacOS"
lib="$steam/Frameworks/Steam Helper.app/Contents/MacOS/libsteam_api.dylib"
if [ ! -f "$lib" ]; then
    echo "libsteam_api.dylib not found in the Steam installation" >&2
    exit 1
fi
pgrep -q steam_osx || { echo "Steam is not running; start it and sign in" >&2; exit 1; }

clang -arch x86_64 -O1 -Wall "$here/probe.c" -o "$out/probe-x86_64"
echo "steam client build: $(defaults read "$steam/../Info.plist" CFBundleVersion 2>/dev/null || echo unknown)"
set +e
perl -e 'alarm shift; exec @ARGV' "${PROBE_TIMEOUT:-30}" \
    arch -x86_64 -e SteamAppId=730 -e SteamGameId=730 "$out/probe-x86_64" "$lib" 2>&1 \
    | grep -E '^(arch|declared app|init|connected to Steam|account|app the client sees|subscribed to the declared app|holds app 730|result):'
status=${PIPESTATUS[0]}
set -e
[ "$status" -eq 142 ] && echo "result: timed out"
echo "exit: $status"
