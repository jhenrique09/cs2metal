# Shared by cs2metal's scripts: where things live, and a few helpers. Sourced.
#
# Everything outside the checkout lives under one directory, ~/cs2-metal:
#
#   bin/    the host program (inside cs2metal.app), the guest's C library and
#           shims, and these scripts
#   data/   steamcmd, the game, what the game writes, the C++ runtime, the baseline
#   logs/   one log per session
#
# CS2METAL_HOME moves it, for tests; nothing else should need to.

CS2METAL_HOME="${CS2METAL_HOME:-$HOME/cs2-metal}"
bin_dir="$CS2METAL_HOME/bin"
data_dir="$CS2METAL_HOME/data"
logs_dir="$CS2METAL_HOME/logs"

# The host program lives in an application bundle, whose Info.plist is what
# tells macOS that it is a game (host-bundle.sh).
host_app="$bin_dir/cs2metal.app"
host_bin="$host_app/Contents/MacOS/cs2metal"
guest_dir="$bin_dir/guest"
game_dir="$data_dir/cs2-linux"
overlay_dir="$data_dir/cs2-linux.overlay"
runtime_dir="$data_dir/sniper"
steamcmd_dir="$data_dir/steamcmd"
baseline="$data_dir/baselines/cs2-linux.json"

# Counter-Strike 2's Steam app.
app_id=730
# steamcmd signs in anonymously by default: the anonymous account holds a licence
# for the app. CS2METAL_STEAM_LOGIN names an account instead; steamcmd then asks
# for its password itself, and no script here sees it.
steam_login="${CS2METAL_STEAM_LOGIN:-anonymous}"

steam_client="$HOME/Library/Application Support/Steam/Steam.AppBundle/Steam/Contents/MacOS"
steam_userdata="$HOME/Library/Application Support/Steam/userdata"

say() { printf 'cs2metal: %s\n' "$*"; }
fail() {
    printf 'cs2metal: %s\n' "$*" >&2
    exit 1
}

steam_running() { pgrep -f "Steam.AppBundle/Steam/Contents/MacOS/steam_osx" > /dev/null; }
game_running() { pgrep -x cs2metal > /dev/null; }

# Converts the game's own icon, a 512-pixel bitmap in the installation, into
# the .icns file named, in the sizes one holds. Nothing of Valve's is shipped
# with cs2metal: the icon is made on this Mac from the owner's copy. Fails,
# having made nothing, where the bitmap is not there.
make_icon() {
    local out="$1" bitmap="$game_dir/game/csgo/resource/game-icon.bmp" set_dir size double
    [ -f "$bitmap" ] || return 1
    set_dir="$(mktemp -d)/icon.iconset"
    mkdir -p "$set_dir"
    for size in 16 32 128 256 512; do
        sips -s format png -z "$size" "$size" "$bitmap" --out "$set_dir/icon_${size}x${size}.png" > /dev/null
    done
    for size in 16 128 256; do
        double=$((size * 2))
        cp "$set_dir/icon_${double}x${double}.png" "$set_dir/icon_${size}x${size}@2x.png"
    done
    iconutil -c icns "$set_dir" -o "$out"
    rm -rf "$(dirname "$set_dir")"
}

# The newest Vulkan SDK's macOS directory, or nothing.
vulkan_sdk() {
    if [ -n "${VULKAN_SDK:-}" ]; then
        echo "$VULKAN_SDK"
    else
        ls -d "$HOME"/VulkanSDK/*/macOS 2> /dev/null | sort -V | tail -1
    fi
}

# Runs steamcmd from its own directory, without a terminal to read from.
steamcmd() { (cd "$steamcmd_dir" && ./steamcmd.sh "$@" < /dev/null); }

# The build of the game that is installed, from steamcmd's own record.
installed_build() {
    sed -n 's/.*"buildid"[[:space:]]*"\([0-9]*\)".*/\1/p' \
        "$game_dir/steamapps/appmanifest_$app_id.acf" 2> /dev/null | head -1
}

# The build Steam offers on the public branch, or nothing if Steam cannot be asked.
offered_build() {
    steamcmd +login anonymous +app_info_update 1 +app_info_print "$app_id" +quit 2> /dev/null |
        awk '/"branches"/ { b = 1 } b && /"public"/ { p = 1 }
             p && /"buildid"/ { gsub(/[^0-9]/, "", $2); print $2; exit }'
}

# Downloads the game, or brings it up to date, and checks every file.
download_game() {
    steamcmd +@sSteamCmdForcePlatformType linux +force_install_dir "$game_dir" \
        +login "$steam_login" +app_update "$app_id" validate +quit
}

# A checksum of the functions the game's own SDL exports. The SDL shim is built
# for exactly that list, so a game update that changes it needs a new build.
sdl_exports() {
    objdump -T "$game_dir/game/bin/linuxsteamrt64/libSDL3.so.0" 2> /dev/null |
        awk '$4 == ".text" && $NF ~ /^SDL_/ { print $NF }' | LC_ALL=C sort -u | shasum -a 256 | cut -d' ' -f1
}
