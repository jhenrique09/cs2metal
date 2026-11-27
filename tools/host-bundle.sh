#!/usr/bin/env bash
# Puts the host program into an application bundle, so that macOS sees the game
# as a game.
#
#   tools/host-bundle.sh <bundle.app> <host-program> [<version>]
#
# A program outside a bundle has no Info.plist, and macOS takes what it knows
# of an application from that file. Game Mode, which gives a full-screen game
# the processor and the GPU ahead of background work, is offered only to an
# application that declares itself a game there (LSApplicationCategoryType, a
# games category, and LSSupportsGameMode). The keys are not inherited by a
# program an application starts, so it is the host itself, the process that
# owns the game's window, that has to live in a bundle that carries them.
#
# The bundle is <bundle.app>/Contents/MacOS/cs2metal with an Info.plist beside
# it. The program is written beside its place and renamed, so that a running
# game keeps the program it started. An icon, if <bundle.app>/Contents/
# Resources/icon.icns is there already or is put there afterwards, is the
# bundle's.
#
# The microphone line is what macOS shows when the game first asks for the
# microphone, for voice chat; an application without one is refused it.
set -euo pipefail

app="${1:?usage: host-bundle.sh <bundle.app> <host-program> [<version>]}"
program="${2:?usage: host-bundle.sh <bundle.app> <host-program> [<version>]}"
version="${3:-0}"

[ -f "$program" ] || {
    echo "host-bundle: no host program at $program" >&2
    exit 1
}
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
cp "$program" "$app/Contents/MacOS/cs2metal.new"
chmod 755 "$app/Contents/MacOS/cs2metal.new"
mv "$app/Contents/MacOS/cs2metal.new" "$app/Contents/MacOS/cs2metal"

cat > "$app/Contents/Info.plist.new" << PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleName</key><string>Counter-Strike 2</string>
    <key>CFBundleDisplayName</key><string>Counter-Strike 2</string>
    <key>CFBundleIdentifier</key><string>local.cs2metal.host</string>
    <key>CFBundleExecutable</key><string>cs2metal</string>
    <key>CFBundleIconFile</key><string>icon</string>
    <key>CFBundlePackageType</key><string>APPL</string>
    <key>CFBundleInfoDictionaryVersion</key><string>6.0</string>
    <key>CFBundleShortVersionString</key><string>$version</string>
    <key>LSApplicationCategoryType</key><string>public.app-category.action-games</string>
    <key>LSSupportsGameMode</key><true/>
    <key>GCSupportsGameMode</key><true/>
    <key>NSHighResolutionCapable</key><true/>
    <key>NSMicrophoneUsageDescription</key><string>Counter-Strike 2 uses the microphone for voice chat.</string>
</dict>
</plist>
PLIST
mv "$app/Contents/Info.plist.new" "$app/Contents/Info.plist"
# Makes Launch Services read the bundle again.
touch "$app"
