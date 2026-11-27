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
# The first launch of a kit starts from the game's own low video preset
# (tools/low-preset.sh), because the settings the game picks for hardware it does
# not know can be heavy for this host. The kit is then marked, in
# <kit>/.cs2metal-first-launch, and no later launch touches the video settings:
# what the owner chooses in the game stays.
#
# The Steam overlay (Shift+Tab, and the pages the game opens in Steam's browser)
# is Steam's own: the macOS client's overlay renderer is inserted into the host
# process, as Steam does for a game it starts, and Steam then starts its overlay
# for the game. It draws on the Metal layer under MoltenVK.
#
# -nojoy is always passed, at the owner's request. In other Source games it keeps
# the joystick system from starting. This build starts no joystick or gamepad
# system either way (SDL is asked for video and audio only), and no effect of the
# option was seen, in client boots 75 and 76 of 27 November 2026.
#
# Environment:
#   CS2METAL_KIT    the directory with cs2-linux/ and sniper/ (default ~/cs2metal-kit)
#   CS2METAL_NO_OVERLAY=1            start without the Steam overlay
#   CS2METAL_KEEP_VIDEO_SETTINGS=1   leave the video settings alone at the first launch too
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

# The graphics device's vendor and device ids, in decimal, as the Vulkan SDK's
# vulkaninfo reports them for MoltenVK; nothing if the SDK is not found.
device_ids() {
    local sdk ids
    sdk="${VULKAN_SDK:-$(ls -d "$HOME"/VulkanSDK/*/macOS 2>/dev/null | sort -V | tail -1)}"
    [ -x "$sdk/bin/vulkaninfo" ] || return 0
    ids="$(VK_DRIVER_FILES="$sdk/share/vulkan/icd.d/MoltenVK_icd.json" \
        "$sdk/bin/vulkaninfo" --summary 2>/dev/null |
        awk '$1 == "vendorID" && !v { v = $3 } $1 == "deviceID" && !d { d = $3 } END { print v, d }')"
    # shellcheck disable=SC2086
    case "$ids" in
        0x*\ 0x*) printf '%d %d\n' $ids ;;
    esac
}

first_launch="$kit/.cs2metal-first-launch"
if [ ! -e "$first_launch" ]; then
    preset=0
    if [ "${CS2METAL_KEEP_VIDEO_SETTINGS:-0}" != 1 ]; then
        echo "launch: first launch of this kit: starting from the game's low video preset"
        # shellcheck disable=SC2046
        "$repo/tools/low-preset.sh" "$HOME/Library/Application Support/Steam/userdata" \
            $(device_ids) || preset=$?
    fi
    # Status 3: there was no settings file to change and none could be made; the
    # game makes one in this launch, and the next launch sets the preset in it.
    if [ "$preset" != 3 ]; then
        date > "$first_launch"
    fi
fi

# The overlay renderer finds its game by these, which the Steam bridge would set
# only once the game has declared its app.
overlay="$HOME/Library/Application Support/Steam/Steam.AppBundle/Steam/Contents/MacOS/gameoverlayrenderer.dylib"
if [ "${CS2METAL_NO_OVERLAY:-0}" != 1 ] && [ -f "$overlay" ]; then
    export DYLD_INSERT_LIBRARIES="$overlay"
    export SteamAppId=730 SteamGameId=730
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
    -- ${mode[@]+"${mode[@]}"} -condebug -nojoy ${engine_args[@]+"${engine_args[@]}"} > "$log" 2>&1
