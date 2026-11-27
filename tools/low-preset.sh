#!/usr/bin/env bash
# Sets the game's video settings to its own low preset, for the first launch.
#
#   tools/low-preset.sh <steam-userdata-dir> [<vendor-id> <device-id>]
#
# The game keeps its video settings per Steam account, in
# <userdata>/<account>/730/local/cfg/cs2_video.txt. For every account directory
# this rewrites the quality settings of that file to the values the game's own
# "low" preset stores, and leaves every other line (resolution, display mode,
# device) as it is. A file that is there is copied first, once, to
# cs2_video.txt.before-cs2metal.
#
# An account without the file gets one only if the graphics device's ids are
# given, in decimal, as Vulkan reports them. The game takes a settings file
# only when it names the file's version and the device it runs on (measured on
# 27 November 2026: without them it discards the file and configures itself);
# the settings the file lacks, it fills in. The new file also asks for the
# full-screen window the game's own configuration chooses, which takes the
# display's size; without it the game opens a 1280 by 720 window.
#
# Exit status 3: no account had the file and no ids were given, so nothing was
# done and the game will configure itself; run this again after that launch.
#
# The launcher runs this once: it marks the kit after the first launch, so
# settings the owner chooses later are never touched.
set -euo pipefail

userdata="${1:?usage: low-preset.sh <steam-userdata-dir> [<vendor-id> <device-id>]}"
vendor="${2:-}"
device="${3:-}"

# The version of the settings file in game build 26879581.
version=17

# The game's low preset, as it stored it on 27 November 2026.
low="setting.shaderquality=0 \
setting.r_texturefilteringquality=0 \
setting.msaa_samples=0 \
setting.r_csgo_cmaa_enable=0 \
setting.videocfg_shadow_quality=0 \
setting.videocfg_dynamic_shadows=0 \
setting.videocfg_texture_detail=0 \
setting.videocfg_particle_detail=0 \
setting.videocfg_ao_detail=0 \
setting.videocfg_parallax_detail=0 \
setting.videocfg_hdr_detail=3 \
setting.videocfg_fsr_detail=3"

# Rewrites the settings in `low` in the file on standard input: a line of one
# of them gets the low value, and those the file lacks go in before its closing
# brace.
rewrite() {
    awk -v low="$low" '
        BEGIN {
            count = split(low, pairs, " ")
            for (i = 1; i <= count; i++) {
                split(pairs[i], pair, "=")
                order[i] = pair[1]
                value[pair[1]] = pair[2]
            }
        }
        function line(key) { return "\t\"" key "\"\t\t\"" value[key] "\"" }
        {
            key = $1
            gsub(/"/, "", key)
            if (key in value) {
                print line(key)
                seen[key] = 1
            } else if ($0 ~ /^}/) {
                for (i = 1; i <= count; i++) {
                    if (!(order[i] in seen)) {
                        print line(order[i])
                    }
                }
                print
            } else {
                print
            }
        }
    '
}

[ -d "$userdata" ] || exit 0
applied=0
skipped=0
for account in "$userdata"/*/; do
    name="$(basename "$account")"
    # Account directories are numbers; 0 holds no account.
    case "$name" in
        0 | *[!0-9]*) continue ;;
    esac
    cfg="${account}730/local/cfg"
    file="$cfg/cs2_video.txt"
    if [ -f "$file" ]; then
        [ -e "$file.before-cs2metal" ] || cp "$file" "$file.before-cs2metal"
        rewrite < "$file" > "$file.cs2metal-new"
    elif [ -n "$vendor" ] && [ -n "$device" ]; then
        mkdir -p "$cfg"
        {
            printf '"video.cfg"\n{\n'
            printf '\t"%s"\t\t"%s"\n' Version "$version" VendorID "$vendor" DeviceID "$device" \
                setting.fullscreen 0 setting.coop_fullscreen 1 setting.nowindowborder 1
            printf '}\n'
        } | rewrite > "$file.cs2metal-new"
    else
        skipped=$((skipped + 1))
        continue
    fi
    mv "$file.cs2metal-new" "$file"
    applied=$((applied + 1))
    echo "low-preset: video settings set to the low preset for account directory $name"
done
if [ "$applied" = 0 ] && [ "$skipped" != 0 ]; then
    echo "low-preset: no settings file yet and no device ids; the game will configure itself" >&2
    exit 3
fi
