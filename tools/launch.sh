#!/usr/bin/env bash
# Starts Counter-Strike 2 (the Linux build in the kit) through cs2metal, with no
# time limit. Quit from the game's menu, or press Ctrl-C here.
#
#   tools/launch.sh                 start the game, offline (-insecure)
#   tools/launch.sh +map de_dust2   extra arguments go to the engine
#   tools/launch.sh -online         start without -insecure (owner only, see below)
#
# -insecure is passed unless -online is given: without it the game can join
# VAC-secured servers through cs2metal, and how Valve Anti-Cheat treats this host
# is unknown. -online is the owner's opt-in, recorded in
# docs/ONLINE_LAUNCH_DECISION.md: use a secondary account. Automated contributors
# never pass it. The option is the launcher's own and does not reach the engine.
#
# Environment:
#   CS2METAL_KIT    the directory with cs2-linux/ and sniper/ (default ~/cs2metal-kit)
#   CS2METAL_*      diagnostics, passed through (e.g. CS2METAL_SDL_TRACE=1)
set -euo pipefail

repo="$(cd "$(dirname "$0")/.." && pwd)"
kit="${CS2METAL_KIT:-$HOME/cs2metal-kit}"
bin="$repo/target/x86_64-apple-darwin/release/cs2metal"
logs="$kit/logs"

online=0
engine_args=()
for arg in "$@"; do
    if [ "$arg" = "-online" ]; then
        online=1
    else
        engine_args+=("$arg")
    fi
done

if [ ! -x "$bin" ]; then
    echo "launch: building cs2metal (release, x86-64)..."
    (cd "$repo" && cargo build --release --target x86_64-apple-darwin -p cs2metal --locked)
fi

if pgrep -f "$bin run" > /dev/null; then
    echo "launch: cs2metal is already running; quit it first." >&2
    exit 1
fi

if ! pgrep -f "Steam.AppBundle/Steam/Contents/MacOS/steam_osx" > /dev/null; then
    echo "launch: Steam for macOS is not running; opening it. Sign in, then run this again." >&2
    open -a Steam || true
    exit 1
fi

if [ "$online" = 1 ]; then
    mode=()
    echo "launch: -online: starting WITHOUT -insecure. VAC-secured servers are" >&2
    echo "launch: reachable and the account signed in to Steam carries the risk." >&2
else
    mode=(-insecure)
fi

mkdir -p "$logs"
log="$logs/cs2-$(date +%Y%m%d-%H%M%S).log"
echo "launch: log at $log"

# The run command's glibc and baseline defaults are relative to the checkout.
# The ${name[@]+...} form keeps an empty array from tripping `set -u` in bash 3.2.
cd "$repo"
exec "$bin" run \
    --game-dir "$kit/cs2-linux" \
    --runtime-dir "$kit/sniper/files" \
    -- ${mode[@]+"${mode[@]}"} -condebug ${engine_args[@]+"${engine_args[@]}"} > "$log" 2>&1
