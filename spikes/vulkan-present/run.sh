#!/usr/bin/env bash
# Builds the F1 probe for x86_64 (the host's architecture, under Rosetta 2) and for
# arm64 (a native control), then runs it with each Vulkan driver of the Vulkan SDK.
# Each run opens a window for about five seconds.
set -euo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
root="$(git -C "$here" rev-parse --show-toplevel)"
out="$root/target/spikes/vulkan-present"
mkdir -p "$out"

sdk="${VULKAN_SDK:-$(ls -d "$HOME"/VulkanSDK/*/macOS 2>/dev/null | sort -V | tail -1)}"
if [ ! -d "$sdk/lib" ]; then
    echo "Vulkan SDK not found; set VULKAN_SDK to its macOS directory" >&2
    exit 1
fi
sdl="$sdk/lib/SDL3.xcframework/macos-arm64_x86_64"
icd="$sdk/share/vulkan/icd.d"

"$sdk/bin/glslc" "$here/triangle.vert" -o "$out/vert.spv"
"$sdk/bin/glslc" "$here/triangle.frag" -o "$out/frag.spv"
for arch in x86_64 arm64; do
    clang -arch "$arch" -O1 -Wall -Wno-missing-braces \
        -I"$sdk/include" -F"$sdl" "$here/probe.c" -o "$out/probe-$arch" \
        -L"$sdk/lib" -lvulkan -framework SDL3 \
        -Wl,-rpath,"$sdk/lib" -Wl,-rpath,"$sdl"
done

timeout_seconds="${PROBE_TIMEOUT:-60}"
echo "machine: $(sysctl -n machdep.cpu.brand_string), macOS $(sw_vers -productVersion) ($(sw_vers -buildVersion))"
echo "sdk: $(basename "$(dirname "$sdk")")"
for driver in MoltenVK libkosmickrisp; do
    for arch in x86_64 arm64; do
        echo "== $driver, $arch"
        set +e
        perl -e 'alarm shift; exec @ARGV' "$timeout_seconds" \
            arch "-$arch" -e "VK_DRIVER_FILES=$icd/${driver}_icd.json" \
            "$out/probe-$arch" "$sdk/lib/libvulkan.1.dylib" "$out/vert.spv" "$out/frag.spv" 2>&1 \
            | grep -E '^(arch|device|driver|pipeline creation ms|swapchain|first frame ms|frames|result):'
        status=${PIPESTATUS[0]}
        set -e
        [ "$status" -eq 142 ] && echo "result: timed out"
        echo "exit: $status"
    done
done
