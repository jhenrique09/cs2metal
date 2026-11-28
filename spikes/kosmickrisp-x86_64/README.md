# KosmicKrisp for x86-64

LunarG's Vulkan SDK ships Mesa's KosmicKrisp driver for arm64 only, so the Vulkan
loader refuses it in the cs2metal host, which is an x86-64 process under Rosetta 2.
This builds it for x86-64 from upstream Mesa:

```bash
brew install meson ninja llvm spirv-tools spirv-llvm-translator libclc
spikes/kosmickrisp-x86_64/build.sh            # or: build.sh <mesa-commit>
VK_DRIVER_FILES=~/cs2-metal/drivers/kosmickrisp-x86_64/kosmickrisp_icd.json \
    ~/cs2-metal/bin/run.sh -offline
```

The script clones Mesa into `~/cs2-metal/src`, applies `lazy-occlusion.patch`,
builds Mesa's compiler tools natively and then the driver with `x86_64.ini`, and
installs the library and a manifest of its own into
`~/cs2-metal/drivers/kosmickrisp-x86_64`. Nothing is installed system-wide.

| File | What it is |
| --- | --- |
| `build.sh` | The two-stage build, and the install |
| `x86_64.ini` | Meson's cross file for x86-64 on an arm64 Mac |
| `lazy-occlusion.patch` | Occlusion queries take an entry of KosmicKrisp's device-wide table when first begun, not when their pool is made. The game makes pools of 16,384, and the third failed against the table's 32,768 entries, which Metal's 256 KiB visibility buffer limit caps |

Switching drivers makes the game replace its pipeline cache under
`~/cs2-metal/data/cs2-linux.overlay/game/csgo/shadercache/vulkan`; copy it aside
first to keep MoltenVK's. The first start on KosmicKrisp stops for seconds at a time
while Metal compiles its shaders.

Measured on 27 November 2026: it runs the game, at 44% fewer frames than MoltenVK
([measurement](../../docs/KOSMICKRISP_X86_64_MEASUREMENT_2026-11-27.md)).
