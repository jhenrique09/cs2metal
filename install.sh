#!/usr/bin/env bash
# Sets up Counter-Strike 2 on this Mac through cs2metal, from this checkout:
# checks what must be installed, downloads the game and its runtime, builds
# everything, and offers a shortcut. Safe to run again: it skips what is done.
#
#   ./install.sh
#
# What it does, in order:
#
# 1. Checks the Mac: Apple silicon, Rosetta 2, the Xcode command line tools,
#    rustup, the Vulkan SDK, and a way to get the guest's C library (Docker,
#    unless the library is already built or given: see build.sh). It says how to
#    install what is missing.
# 2. Steam for macOS: tells you to install it and sign in, and waits. Do NOT
#    install Counter-Strike 2 from that client: it would download the game
#    without the Linux programs this host runs. Steam itself is needed, as the
#    Steam the game talks to.
# 3. steamcmd, Valve's command-line downloader, into ~/cs2-metal/data/steamcmd.
# 4. The Linux build of the game, with steamcmd, into ~/cs2-metal/data/cs2-linux:
#    about 60 GB to download, 71 GB on disk.
# 5. The C++ runtime of the Steam Linux Runtime 3.0 ("sniper"), 305 MB, of which
#    two libraries are kept, into ~/cs2-metal/data/sniper.
# 6. ./build.sh: the host program, the guest's patched C library, and the shims,
#    installed to ~/cs2-metal/bin.
# 7. The integrity baseline: a checksum of every game file, which cs2metal
#    verifies before each start.
# 8. Asks whether to make an application shortcut, and whether to add it to Steam.
#
# steamcmd signs in anonymously, which is enough for this game. To download as
# your own account instead, set CS2METAL_STEAM_LOGIN=<account>: steamcmd then
# asks for the password and the Steam Guard code itself. No script here sees them.
set -euo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
. "$here/tools/common.sh"
cd "$here"

step() { printf '\n== %s\n' "$*"; }
ask() {
    local answer
    printf '%s [y/N] ' "$1"
    read -r answer || answer=""
    case "$answer" in y | Y | yes | YES) return 0 ;; *) return 1 ;; esac
}

step "1. Checking this Mac"
missing=0
need() {
    printf '   missing: %s\n            %s\n' "$1" "$2"
    missing=1
}
[ "$(uname -s)" = Darwin ] && [ "$(uname -m)" = arm64 ] ||
    need "an Apple silicon Mac" "cs2metal runs the game's x86-64 code under Rosetta 2"
arch -x86_64 /usr/bin/true 2> /dev/null ||
    need "Rosetta 2" "softwareupdate --install-rosetta --agree-to-license"
xcode-select -p > /dev/null 2>&1 ||
    need "the Xcode command line tools" "xcode-select --install"
command -v cargo > /dev/null ||
    need "rustup" "curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh"
command -v python3 > /dev/null ||
    need "python3" "it comes with the Xcode command line tools"
# The guest's C library is the one thing built in a Linux container, and only
# when it is neither built already nor given (see the top of build.sh).
glibc_at_hand() {
    [ -n "${CS2METAL_GLIBC_FROM:-}" ] || [ -n "${CS2METAL_BUILD_HOST:-}" ] ||
        (cd target/guest-glibc 2> /dev/null &&
            shasum -a 256 -c --quiet "$here/guest/glibc/SHA256SUMS" > /dev/null 2>&1)
}
if ! glibc_at_hand; then
    if ! command -v docker > /dev/null; then
        need "Docker, to build the guest's C library once" \
            "install OrbStack (https://orbstack.dev) or Docker Desktop and start it; or set CS2METAL_GLIBC_FROM (see build.sh)"
    elif ! docker info > /dev/null 2>&1; then
        need "Docker, running" "start OrbStack or Docker Desktop"
    fi
fi
sdk="$(vulkan_sdk)"
if [ -z "$sdk" ] || [ ! -f "$sdk/lib/libMoltenVK.dylib" ] || [ ! -d "$sdk/lib/SDL3.xcframework" ]; then
    need "the Vulkan SDK for macOS, with SDL3" \
        "download the installer from https://vulkan.lunarg.com/sdk/home and keep its default location, ~/VulkanSDK"
fi
[ "$missing" = 0 ] || fail "install what is missing, then run ./install.sh again"
echo "   everything needed is here"

step "2. Steam for macOS"
if [ ! -f "$steam_client/steamclient.dylib" ]; then
    cat << 'TEXT'
   Steam is not installed. Download it from https://store.steampowered.com/about/
   drag it to Applications, start it once, and sign in.
TEXT
fi
cat << 'TEXT'
   Steam must be running and signed in whenever you play.
   Do NOT install Counter-Strike 2 from Steam for macOS: this script downloads
   the Linux build, which is the one cs2metal runs.
TEXT
until [ -f "$steam_client/steamclient.dylib" ] && steam_running; do
    printf '   Start Steam and sign in, then press Return (Ctrl-C to stop here). '
    read -r _ || exit 1
done
echo "   Steam is running"

mkdir -p "$bin_dir" "$data_dir" "$logs_dir"

step "3. steamcmd"
if [ -x "$steamcmd_dir/steamcmd.sh" ]; then
    echo "   already in $steamcmd_dir"
else
    mkdir -p "$steamcmd_dir"
    curl -sSfL https://steamcdn-a.akamaihd.net/client/installer/steamcmd_osx.tar.gz |
        tar -xz -C "$steamcmd_dir"
    echo "   downloaded to $steamcmd_dir"
fi

step "4. The Linux build of Counter-Strike 2"
if [ -f "$game_dir/game/bin/linuxsteamrt64/cs2" ] && [ -n "$(installed_build)" ]; then
    echo "   build $(installed_build) is already in $game_dir"
else
    echo "   About 60 GB to download and 71 GB on disk, into $game_dir."
    echo "   It can be interrupted and run again: steamcmd resumes."
    ask "   Download it now?" || fail "the game is needed for everything after this step"
    download_game
    file "$game_dir/game/bin/linuxsteamrt64/cs2" | grep -q "ELF 64-bit.*x86-64" ||
        fail "the download did not leave the Linux program in $game_dir"
    echo "   downloaded build $(installed_build)"
fi

step "5. The C++ runtime (Steam Linux Runtime 3.0)"
libs="$runtime_dir/files/lib/x86_64-linux-gnu"
if [ -f "$libs/libstdc++.so.6" ] && [ -f "$libs/libgcc_s.so.1" ]; then
    echo "   already in $runtime_dir"
else
    mkdir -p "$runtime_dir"
    base=https://repo.steampowered.com/steamrt3/images/latest-public-stable
    archive='com.valvesoftware.SteamRuntime.Platform-amd64,i386-sniper-runtime.tar.gz'
    (
        cd "$runtime_dir"
        curl -fL --http1.1 --retry 5 --retry-all-errors -C - -O "$base/$archive"
        curl -fsSL --retry 5 -O "$base/SHA256SUMS"
        grep -F "$archive" SHA256SUMS | shasum -a 256 -c -
        tar -xzf "$archive" --include '*/x86_64-linux-gnu/libstdc++.so.6*' \
            --include '*/x86_64-linux-gnu/libgcc_s.so.1'
        rm -f "$archive" SHA256SUMS
    )
    [ -f "$libs/libstdc++.so.6" ] || fail "the runtime's libraries are not in $libs"
    echo "   downloaded to $runtime_dir"
fi

step "6. Building cs2metal"
./build.sh

step "7. The integrity baseline"
if [ -f "$baseline" ] && "$host_bin" baseline verify "$game_dir" --baseline "$baseline" --quick > /dev/null 2>&1; then
    echo "   the installation matches its baseline"
else
    mkdir -p "$(dirname "$baseline")"
    "$host_bin" baseline record "$game_dir" --output "$baseline" --force
fi

step "8. A shortcut"
if ask "   Make an application, Counter-Strike 2, in ~/Applications?"; then
    if ask "   Also add it to Steam's library? (Steam must be quit for that)"; then
        if steam_running; then
            echo "   Quit Steam now (Steam menu, Quit Steam)."
            until ! steam_running; do
                printf '   Press Return once Steam has quit. '
                read -r _ || exit 1
            done
        fi
        "$bin_dir/create-shortcut.sh" --steam
        echo "   Start Steam again before you play."
    else
        "$bin_dir/create-shortcut.sh"
    fi
fi

cat << TEXT

Done. Start the game with:

    $bin_dir/run.sh

It starts online, at your own risk: Valve does not support this, and how Valve
Anti-Cheat treats it is unknown. For bots only, start it with -offline.
Logs go to $logs_dir.
The first start stays black for a few minutes while the game's shaders are
compiled for Metal, once. It is not stuck; let it finish.
TEXT
