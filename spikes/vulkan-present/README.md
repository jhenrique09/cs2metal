# Vulkan presentation probe (roadmap step F1)

Answers open question Q-6 for the renderer path: can a Vulkan driver run inside an
x86-64 process under Rosetta 2, which is what the cs2metal host is, and present to a
window? The probe takes the same route the game's renderer will take on the host:
an SDL3 window, `SDL_Vulkan_CreateSurface`, a swapchain.

```bash
spikes/vulkan-present/run.sh
```

It needs LunarG's Vulkan SDK for macOS (roadmap step A6), found under
`~/VulkanSDK/<version>/macOS` or through `VULKAN_SDK`. The SDK provides the
universal Vulkan loader, MoltenVK, KosmicKrisp, `glslc`, and a universal
`SDL3.framework`. The script compiles the two shaders and builds `probe.c` for
x86-64 and, as a native control, for arm64, into `target/spikes/`. It then runs each
build with each driver, chosen through the loader's `VK_DRIVER_FILES`. Every run
opens a window for about five seconds.

For each run the probe reports:

| Line | Meaning |
| --- | --- |
| `arch` | The process architecture, and whether Rosetta 2 translates it |
| `device`, `driver` | What the loader found |
| `pipeline creation ms` | Two shader modules and one graphics pipeline, from SPIR-V: the driver's shader translation, including its first-use setup |
| `swapchain` | Size, image count, present mode |
| `first frame ms` | Acquire, clear, submit, present, and wait, for the first image |
| `frames` | Mean and worst time over 300 presented frames, in FIFO mode, so bounded by the display's refresh rate |
| `result` | `presented`, or the step that failed |

The results and their interpretation are recorded in
[the Vulkan driver measurement](../../docs/VULKAN_DRIVER_MEASUREMENT_2026-11-27.md).
