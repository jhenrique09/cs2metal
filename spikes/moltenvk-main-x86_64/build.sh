#!/usr/bin/env bash
# Builds upstream MoltenVK for x86-64 on an Apple silicon Mac, so that it loads
# in the cs2metal host under Rosetta 2, and installs it with a driver manifest.
#
# Needs: Xcode and the Vulkan SDK. Then: spikes/moltenvk-main-x86_64/build.sh [commit]
# Use:   VK_DRIVER_FILES=~/cs2-metal/drivers/moltenvk-main-x86_64/MoltenVK_icd.json
set -euo pipefail

work="${MVK_WORK:-$HOME/cs2-metal/src}"
out="${MVK_OUT:-$HOME/cs2-metal/drivers/moltenvk-main-x86_64}"
commit="${1:-5189f8de86ec365860f63bf1da736815cfaea5cd}"
moltenvk="$work/MoltenVK"

if ! xcodebuild -version > /dev/null 2>&1; then
    if [ -d /Applications/Xcode.app/Contents/Developer ]; then
        export DEVELOPER_DIR=/Applications/Xcode.app/Contents/Developer
    else
        echo "Xcode is required; xcode-select points only to the Command Line Tools" >&2
        exit 1
    fi
fi

mkdir -p "$work" "$out"
if [ ! -d "$moltenvk/.git" ]; then
    git clone --filter=blob:none https://github.com/KhronosGroup/MoltenVK.git "$moltenvk"
fi
git -C "$moltenvk" fetch --depth 1 origin "$commit"
git -C "$moltenvk" checkout --quiet --force "$commit"

(
    cd "$moltenvk"
    ./fetchDependencies --macos --keep-cache
    xcodebuild build -quiet -project MoltenVKPackaging.xcodeproj \
        -scheme "MoltenVK Package (macOS only)" -configuration Release \
        -destination "generic/platform=macOS" ARCHS=x86_64 ONLY_ACTIVE_ARCH=NO
)

lib="$out/libMoltenVK.dylib"
cp -p "$moltenvk/Package/Release/MoltenVK/dynamic/dylib/macOS/libMoltenVK.dylib" "$lib"
[ "$(lipo -archs "$lib")" = x86_64 ] || {
    echo "the built driver is not x86-64 only" >&2
    exit 1
}
cat > "$out/MoltenVK_icd.json" << JSON
{
    "file_format_version": "1.0.0",
    "ICD": {
        "library_path": "$lib",
        "api_version": "1.4.0",
        "is_portability_driver": true
    }
}
JSON
printf '%s\n' "$commit" > "$out/MOLTENVK_COMMIT"
echo "installed $lib ($(lipo -archs "$lib"))"
