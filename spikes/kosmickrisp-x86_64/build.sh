#!/usr/bin/env bash
# Builds Mesa's KosmicKrisp Vulkan driver for x86_64 on an arm64 Mac, so that it
# loads in the cs2metal host under Rosetta 2, and installs it with a driver
# manifest of its own. Two stages, as in shadexternals/mesa-kosmickrisp: Mesa's
# compiler tools natively, then the driver cross-compiled with them.
#
# Needs: brew install meson ninja llvm spirv-tools spirv-llvm-translator libclc
# Then:  spikes/kosmickrisp-x86_64/build.sh [mesa-commit]
# Use:   VK_DRIVER_FILES=~/cs2-metal/drivers/kosmickrisp-x86_64/kosmickrisp_icd.json
set -euo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
work="${KK_WORK:-$HOME/cs2-metal/src}"
out="${KK_OUT:-$HOME/cs2-metal/drivers/kosmickrisp-x86_64}"
commit="${1:-c51e70c3ff1e410cc4a9507fca5f6ba491be68e1}"
mesa="$work/mesa"

mkdir -p "$work" "$out"
if [ ! -d "$mesa/.git" ]; then
    git clone https://gitlab.freedesktop.org/mesa/mesa.git "$mesa"
fi
git -C "$mesa" fetch --depth 1 origin "$commit"
git -C "$mesa" checkout --quiet --force "$commit"
git -C "$mesa" apply "$here/lazy-occlusion.patch"

if [ ! -x "$work/venv/bin/python" ]; then
    python3 -m venv "$work/venv"
    "$work/venv/bin/pip" install --quiet mako packaging pyyaml
fi
export PATH="$work/venv/bin:/opt/homebrew/opt/llvm/bin:$PATH"
export LLVM_CONFIG=/opt/homebrew/opt/llvm/bin/llvm-config
common=(-Dplatforms=macos -Dvulkan-drivers=kosmickrisp -Dgallium-drivers=
    -Dopengl=false -Dzstd=disabled)

tools="$work/kk/build-tools"
[ -d "$tools" ] || meson setup "$tools" "$mesa" --buildtype=release \
    -Dinstall-mesa-clc=true -Dinstall-precomp-compiler=true "${common[@]}"
meson compile -C "$tools" mesa_clc vtn_bindgen2 kk_clc

export PATH="$tools/src/compiler/clc:$tools/src/compiler/spirv:$tools/src/kosmickrisp/clc:$PATH"
export MACOSX_DEPLOYMENT_TARGET=26.0
target="$work/kk/build-x86_64"
[ -d "$target" ] || meson setup "$target" "$mesa" --buildtype=release \
    --cross-file="$here/x86_64.ini" --prefer-static \
    -Dmesa-clc=system -Dprecomp-compiler=system -Dspirv-tools=disabled "${common[@]}"
meson compile -C "$target" vulkan_kosmickrisp kosmickrisp_mesa_icd

# Mesa's zlib subproject leaves the library at @rpath, which nothing sets: point
# it at the system's, which is universal, and sign the result again.
lib="$out/libvulkan_kosmickrisp.dylib"
cp "$target/src/kosmickrisp/vulkan/libvulkan_kosmickrisp.dylib" "$lib"
install_name_tool -change @rpath/libz.1.dylib /usr/lib/libz.1.dylib "$lib" 2> /dev/null
codesign --force --sign - "$lib" 2> /dev/null
api="$(sed -n 's/.*"api_version": "\(.*\)".*/\1/p' \
    "$target"/src/kosmickrisp/vulkan/kosmickrisp_mesa_icd.*.json)"
cat > "$out/kosmickrisp_icd.json" << JSON
{
    "ICD": {
        "api_version": "$api",
        "library_path": "./libvulkan_kosmickrisp.dylib"
    },
    "file_format_version": "1.0.1"
}
JSON
echo "$commit" > "$out/MESA_COMMIT"
echo "installed $lib ($(lipo -archs "$lib"))"
