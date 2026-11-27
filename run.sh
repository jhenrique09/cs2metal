#!/usr/bin/env bash
# Starts Counter-Strike 2 through cs2metal: checks for a game update, applies it
# if there is one, and starts the game. Quit from the game's menu, or press
# Ctrl-C here; either way the game shuts itself down.
#
#   run.sh                 start the game, online
#   run.sh +map de_dust2   extra arguments go to the engine
#   run.sh -offline        start with -insecure: bots and unsecured servers only
#
# Online is the default, by the owner's decision of 27 November 2026
# (docs/ONLINE_DEFAULT_DECISION.md): the game starts as Steam starts it, and can
# join servers secured by Valve Anti-Cheat. Valve does not support this host and
# how Valve Anti-Cheat treats it is unknown, so that is at your own risk: play
# online only with an account you can afford to lose. -offline passes -insecure
# to the engine, which keeps the game off those servers. Automated contributors
# always pass -offline. The option is this script's own and does not reach the
# engine; -online is accepted and changes nothing.
#
# Before the game starts:
#
# - Steam. If Steam for macOS is not running it is opened, and given half a
#   minute to sign in with the account it remembers. If it asks for a password
#   instead, sign in and start the game again.
# - Updates. steamcmd is asked, anonymously, which build Steam offers. If it is
#   not the installed one, steamcmd downloads it, every game file is hashed again
#   into the integrity baseline, and the SDL shim is checked against the new
#   game's SDL: if that changed, the shims must be built again (./build.sh in the
#   checkout) and the game is not started.
# - The first launch starts from the game's own low video preset, with CMAA2
#   anti-aliasing on (low-preset.sh),
#   because what the game picks for hardware it does not know can be heavy for
#   this host. The installation is then marked, in data/first-launch, and no later
#   launch touches the video settings.
# - The Steam overlay (Shift+Tab, and the pages the game opens in Steam's browser)
#   is Steam's own: the macOS client's overlay renderer is inserted into the host
#   process, as Steam does for a game it starts.
# - The opening movie is left out, at the owner's request: under this host it
#   stands still on single frames while the menu loads behind it. The game
#   ignores -novid, so the host gives it half a second of black in its place.
# - -nojoy is passed, at the owner's request; no effect of it was seen.
#
# Environment:
#   CS2METAL_STEAM_WAIT=<seconds>    how long to give Steam to sign in when this
#                                    script had to open it (default 30)
#   CS2METAL_NO_UPDATE=1             do not ask Steam for updates
#   CS2METAL_STEAM_LOGIN=<account>   update as that Steam account, not anonymously
#   CS2METAL_KEEP_VIDEO_SETTINGS=1   leave the video settings alone at the first launch too
#   CS2METAL_NO_OVERLAY=1            start without the Steam overlay
#   CS2METAL_KEEP_INTRO=1            play the opening movie
#   CS2METAL_*                       diagnostics, passed through (e.g. CS2METAL_SDL_TRACE=1)
set -euo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
# In the checkout the helpers are in tools/; installed, they sit beside this script.
if [ -f "$here/tools/common.sh" ]; then helpers="$here/tools"; else helpers="$here"; fi
. "$helpers/common.sh"

online=1
engine_args=()
for arg in "$@"; do
    case "$arg" in
        -offline) online=0 ;;
        -online) online=1 ;;
        *) engine_args+=("$arg") ;;
    esac
done

[ -x "$host_bin" ] || fail "cs2metal is not installed in $bin_dir: run ./install.sh, or ./build.sh, in the checkout"
[ -d "$game_dir/game" ] || fail "the game is not in $game_dir: run ./install.sh in the checkout"
game_running && fail "the game is already running; quit it first"

# The game talks to Steam for macOS, which must be running and signed in. If it
# is not running it is opened here, as its icon would open it, and given time to
# sign in; nothing here signs in or sees how Steam does. No sign of "signed in"
# is read from Steam's files, so the wait is a fixed one.
if ! steam_running; then
    say "Steam for macOS is not running; opening it"
    open -a Steam || fail "could not open Steam: install Steam for macOS and sign in once"
    waited=0
    until steam_running; do
        [ "$waited" -lt 120 ] || fail "Steam did not start in two minutes; start it, sign in, and run this again"
        sleep 1
        waited=$((waited + 1))
    done
    settle="${CS2METAL_STEAM_WAIT:-30}"
    say "waiting $settle seconds for Steam to sign in"
    sleep "$settle"
fi

update_game() {
    [ "${CS2METAL_NO_UPDATE:-0}" = 1 ] && return 0
    [ -x "$steamcmd_dir/steamcmd.sh" ] || {
        say "steamcmd is not installed; not checking for updates"
        return 0
    }
    local have want
    have="$(installed_build)"
    want="$(offered_build || true)"
    if [ -z "$want" ]; then
        say "could not ask Steam for updates; starting build ${have:-unknown}"
        return 0
    fi
    if [ "$have" = "$want" ]; then
        say "the game is up to date (build $have)"
        return 0
    fi
    say "updating the game from build ${have:-unknown} to $want"
    download_game || fail "steamcmd could not update the game"
    say "recording the integrity baseline of build $(installed_build)"
    "$host_bin" baseline record "$game_dir" --output "$baseline" --force
    if [ -f "$guest_dir/SDL_EXPORTS" ] && [ "$(sdl_exports)" != "$(cat "$guest_dir/SDL_EXPORTS")" ]; then
        fail "this update changed the game's SDL: build the shims again with ./build.sh in the checkout"
    fi
}
update_game

if [ "$online" = 1 ]; then
    mode=()
    say "starting online: VAC-secured servers are reachable, at your own risk."
    say "For bots and unsecured servers only, start with -offline."
else
    mode=(-insecure)
    say "starting offline (-insecure)"
fi

# The graphics device's vendor and device ids, in decimal, as the Vulkan SDK's
# vulkaninfo reports them for MoltenVK; nothing if the SDK is not found.
device_ids() {
    local sdk ids
    sdk="$(vulkan_sdk)"
    [ -x "$sdk/bin/vulkaninfo" ] || return 0
    ids="$(VK_DRIVER_FILES="$sdk/share/vulkan/icd.d/MoltenVK_icd.json" \
        "$sdk/bin/vulkaninfo" --summary 2> /dev/null |
        awk '$1 == "vendorID" && !v { v = $3 } $1 == "deviceID" && !d { d = $3 } END { print v, d }')"
    # shellcheck disable=SC2086
    case "$ids" in
        0x*\ 0x*) printf '%d %d\n' $ids ;;
    esac
}

first_launch="$data_dir/first-launch"
if [ ! -e "$first_launch" ]; then
    say "first launch: the screen stays black for a few minutes while the game's"
    say "shaders are compiled for Metal, once. It is not stuck; let it finish."
    preset=0
    if [ "${CS2METAL_KEEP_VIDEO_SETTINGS:-0}" != 1 ]; then
        say "first launch: starting from the game's low video preset"
        # shellcheck disable=SC2046
        "$helpers/low-preset.sh" "$steam_userdata" $(device_ids) || preset=$?
    fi
    # Status 3: there was no settings file to change and none could be made; the
    # game makes one in this launch, and the next launch sets the preset in it.
    if [ "$preset" != 3 ]; then
        date > "$first_launch"
    fi
fi

# The overlay renderer finds its game by these, which the Steam bridge would set
# only once the game has declared its app.
overlay="$steam_client/gameoverlayrenderer.dylib"
if [ "${CS2METAL_NO_OVERLAY:-0}" != 1 ] && [ -f "$overlay" ]; then
    export DYLD_INSERT_LIBRARIES="$overlay"
    export SteamAppId="$app_id" SteamGameId="$app_id"
fi

if [ "${CS2METAL_KEEP_INTRO:-0}" != 1 ]; then
    export CS2METAL_NO_INTRO=1
fi

mkdir -p "$logs_dir"
log="$logs_dir/cs2-$(date +%Y%m%d-%H%M%S).log"
say "log at $log"

# The ${name[@]+...} form keeps an empty array from tripping `set -u` in bash 3.2.
exec "$host_bin" run \
    --game-dir "$game_dir" \
    --runtime-dir "$runtime_dir/files" \
    --glibc "$guest_dir" \
    --baseline "$baseline" \
    --overlay-dir "$overlay_dir" \
    -- ${mode[@]+"${mode[@]}"} -condebug -nojoy ${engine_args[@]+"${engine_args[@]}"} > "$log" 2>&1
