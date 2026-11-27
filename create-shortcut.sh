#!/usr/bin/env bash
# Creates an application that starts the game, with the game's icon, and can add
# it to Steam's library.
#
#   create-shortcut.sh            make ~/Applications/Counter-Strike 2.app
#   create-shortcut.sh --steam    also add it to Steam as a non-Steam game
#   create-shortcut.sh --offline  make an application that starts run-offline.sh
#
# The application is a small bundle whose program runs ~/cs2-metal/bin/run.sh, so
# it shows in Finder, Launchpad, Spotlight, and the Dock under the game's name.
# It declares itself a game that supports Game Mode, as the host's own bundle
# does (host-bundle.sh): the game is one process from this application's start
# to the host, each script replacing itself with the next.
# Like run.sh, it opens Steam first if Steam is not running, and starts the game
# online; --offline makes a second application, Counter-Strike 2 (offline), that
# starts it with -insecure.
# Its icon is converted on this Mac from the game's own icon file,
# game/csgo/resource/game-icon.bmp in the installation: nothing of Valve's is
# shipped with cs2metal.
#
# --steam adds the application to the list of non-Steam games of every Steam
# account on this Mac (steam_shortcuts.py), which is a file Steam reads when it
# starts and writes when it quits: quit Steam first. The file is copied, once,
# to shortcuts.vdf.before-cs2metal beside it. This is the one place where
# cs2metal changes a file of Steam's, by the owner's decision of 27 November
# 2026 (docs/STEAM_SHORTCUT_DECISION.md).
#
# Environment:
#   CS2METAL_APP_DIR   where the application goes (default ~/Applications)
set -euo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
if [ -f "$here/tools/common.sh" ]; then helpers="$here/tools"; else helpers="$here"; fi
. "$helpers/common.sh"

steam=0
script="run.sh"
name="Counter-Strike 2"
for arg in "$@"; do
    case "$arg" in
        --steam) steam=1 ;;
        --offline)
            script="run-offline.sh"
            name="Counter-Strike 2 (offline)"
            ;;
        *) fail "unknown option $arg (see the top of create-shortcut.sh)" ;;
    esac
done

[ -x "$bin_dir/$script" ] || fail "$bin_dir/$script is not installed: run ./build.sh in the checkout"
if [ "$steam" = 1 ] && steam_running; then
    fail "Steam is running: quit Steam, run this again, then start Steam"
fi

app="${CS2METAL_APP_DIR:-$HOME/Applications}/$name.app"
rm -rf "$app"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"

cat > "$app/Contents/MacOS/launch" << LAUNCH
#!/bin/bash
# Made by cs2metal's create-shortcut.sh.
exec "$bin_dir/$script" "\$@"
LAUNCH
chmod +x "$app/Contents/MacOS/launch"

# The icon: the game's own 512-pixel bitmap, in the sizes an .icns holds.
icon=""
if make_icon "$app/Contents/Resources/icon.icns"; then
    icon="$app/Contents/Resources/icon.icns"
else
    say "no icon: the game's icon file is not in $game_dir"
fi

identifier="local.cs2metal.$(echo "$script" | tr -cd 'a-z')"
cat > "$app/Contents/Info.plist" << PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleName</key><string>$name</string>
    <key>CFBundleDisplayName</key><string>$name</string>
    <key>CFBundleIdentifier</key><string>$identifier</string>
    <key>CFBundleExecutable</key><string>launch</string>
    <key>CFBundleIconFile</key><string>icon</string>
    <key>CFBundlePackageType</key><string>APPL</string>
    <key>CFBundleShortVersionString</key><string>$(cat "$bin_dir/VERSION" 2> /dev/null || echo 0)</string>
    <key>LSApplicationCategoryType</key><string>public.app-category.action-games</string>
    <key>LSSupportsGameMode</key><true/>
    <key>GCSupportsGameMode</key><true/>
    <key>NSHighResolutionCapable</key><true/>
</dict>
</plist>
PLIST
# Makes Finder and the Dock read the new bundle and its icon now.
touch "$app"
say "made $app"

if [ "$steam" = 1 ]; then
    "$helpers/steam_shortcuts.py" add --name "$name" --exe "$app" --icon "$icon" --userdata "$steam_userdata"
    say "start Steam: the game is in its library, under non-Steam games"
fi
