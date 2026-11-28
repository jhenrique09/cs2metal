# MoltenVK from its main branch, for x86-64

The cs2metal host uses the MoltenVK of LunarG's Vulkan SDK, a release. This builds
MoltenVK from a commit of its repository instead, for x86-64 only, so that a newer
driver can be measured in the game:

```bash
spikes/moltenvk-main-x86_64/build.sh            # or: build.sh <moltenvk-commit>
VK_DRIVER_FILES=~/cs2-metal/drivers/moltenvk-main-x86_64/MoltenVK_icd.json \
    ~/cs2-metal/bin/run.sh -offline
```

It needs Xcode, not only the Command Line Tools. The script clones MoltenVK into
`~/cs2-metal/src`, fetches its dependencies, builds the macOS package with
`ARCHS=x86_64`, and installs the library and a manifest of its own into
`~/cs2-metal/drivers/moltenvk-main-x86_64`. Nothing is installed system-wide. Two
builds of the same commit gave the same library, byte for byte.

The game kept its video settings across the switch, which it does not between
MoltenVK and KosmicKrisp. Its pipeline cache it shares between the two builds, so copy
`~/cs2-metal/data/cs2-linux.overlay/game/csgo/shadercache/vulkan` aside first.

Measured on 27 and 28 November 2026: commit `5189f8de`, which calls itself 1.4.3,
presented 4% more frames than 1.4.2 over eight alternating boots, inside the
scatter of either
([measurement](../../docs/MOLTENVK_MAIN_AND_SETTINGS_MEASUREMENT_2026-11-28.md)).
The SDK's release stays the host's driver.
