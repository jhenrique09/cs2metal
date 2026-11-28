# MoltenVK's main branch, argument buffers, and video settings, 28 November 2026

## Status

Measured in the night of 27 to 28 November 2026 on the owner's Apple M1 Max, macOS
27.2.1, the game build 26879581, with the owner's video settings: a full screen
window, 1920 by 1080, CMAA2, and FSR on (`videocfg_fsr_detail` 4). It follows the
[KosmicKrisp measurement](KOSMICKRISP_X86_64_MEASUREMENT_2026-11-27.md), and asks
of MoltenVK what that asked of the other driver: is there a frame rate to gain.
Every boot was offline, with `-insecure`, under the authorization in `AGENTS.md`.
Not edited after the fact.

## Questions

1. Does MoltenVK from its main branch present more frames than the SDK's 1.4.2?
2. Does MoltenVK without Metal argument buffers?
3. Which video setting gains frames?
4. Which thread, or the GPU, limits the frame?

## Method

- **The driver.** `spikes/moltenvk-main-x86_64/build.sh` builds MoltenVK at commit
  `5189f8de` for x86-64. It logs itself as 1.4.3; the SDK's logs nothing at the
  default level, which is how the boots are told apart afterwards.
- **Frames.** Boots of 110 seconds on `de_dust2` with bots and nobody at the
  keyboard, presents a second over the last minute from
  `CS2METAL_VULKAN_TRACE=pacing`, and the process's processor time from its 45th
  to its 105th second. Eight boots for the driver, in the orders ABBA and BAAB;
  each driver's copy of the game's pipeline cache put back before its boot.
- **Argument buffers.** One boot with `MVK_CONFIG_USE_METAL_ARGUMENT_BUFFERS=0`.
- **Settings.** One boot each of the owner's settings and of three changes to
  them, in that order, with `CS2METAL_THREAD_TRACE=1`, and the GPU's utilization
  read from the I/O registry every five seconds for the last three.

## Results

### The driver

| Presents a second, last minute | Boots | Mean | Processor time a present |
| --- | --- | --- | --- |
| MoltenVK 1.4.2, the SDK's | 87.9, 89.5, 93.1, 86.5 | 89.3 | 36.6 ms |
| MoltenVK main, `5189f8de` | 94.3, 92.9, 89.5, 94.7 | 92.9 | 34.3 ms |

- **Main is 4% ahead, and the ranges overlap**: its slowest boot is below 1.4.2's
  fastest. One configuration scattered from 73 to 97 the night before. (not shown
  either way)
- **Not enough to carry a driver of our own.** The bar set for KosmicKrisp was 10
  to 15%.

### Argument buffers

Without them the game does not run: Metal refuses to compile the fragment shaders
of its pipelines, and the boot presented 23 frames in ten seconds. (CONFIRMED, one
boot) The renderer's descriptor arrays are the likely reason. (INFERRED)

### Video settings

| Boot, in order | Presents a second | Processor time a present | Presents over twice the median | GPU busy |
| --- | --- | --- | --- | --- |
| The owner's settings | 94.3 | 31.7 ms | 15 | not recorded |
| FSR off (`videocfg_fsr_detail` 0) | 87.6 | 38.7 ms | 10 | 93% (87 to 98) |
| Shadow quality 0 | 85.3 | 42.4 ms | 33 | 82% (72 to 91) |
| Shadows, particles, textures, and filtering at 0 | 84.3 | 38.7 ms | 21 | 88% (77 to 93) |

- **No change gained frames.** (CONFIRMED for these four boots)
- **That is not a ranking.** Lower settings cannot cost frames, and here they
  did: each changed boot was the first with its settings, so it made pipelines
  the others had, and its numbers are inside the scatter besides. One boot each
  shows that no setting brings a large gain, and no more.
- **FSR off loads the GPU most**: busy 93%, at 7% fewer frames, which is inside
  the scatter too.

### Threads

Shares of one processor over the last minute of the four settings boots:

| Thread, by the game's name | Share |
| --- | --- |
| `main` | 52 to 61% |
| `VKRenderThread`, the busiest of four | 55 to 69% |
| `VKRenderThread`, each of the other three | 7 to 13% |
| `GlobPool`, each of seven | 19 to 25% |
| All threads | 3.0 to 3.8 processors |

**No thread is near a whole processor**, and the GPU is the busiest part at 82 to
93%. In the owner's online match of 27 November it was busy 84 to 92%. (CONFIRMED
for these boots)

## Interpretation

- **Nothing measured so far raises MoltenVK's frame rate**: not its main branch,
  not its MTLEvent semaphores, not a lower setting.
- **The frame is not held by one thread.** The main thread, the render thread,
  and the GPU each work a little over half to nine tenths of the time and wait
  for one another the rest. What is left to gain is in those waits and in the
  GPU's work, not in one hot loop. The samples of 27 November, which had the main
  thread waiting for its own jobs a quarter of its time, say the same of it.
- **The offline match is not the online one.** Its main thread also runs the
  server for the bots. A match online, with the thread trace on, is still the
  measurement that is missing.

## Open

- A second round of the settings, with every variant's pipelines made, before any
  setting is called worth or not worth its cost.
- An online match with `CS2METAL_THREAD_TRACE=1`. Only the owner can play one.
- **Uneven text in the full screen window.** At 1920 by 1080 the game's render
  targets are 1920 by 1080 and the window's surface is 2560 by 1440, so the
  finished frame, its interface with it, is stretched by four thirds. In a
  screenshot the same letter sits at the same height wherever it appears, and
  round letters sit 0.7 to 1.4 pixels lower than flat ones: differences of half a
  pixel to one before the stretch. The game loads its own FreeType, fontconfig,
  and Pango, as on Linux. A screenshot at 2560 by 1440 would settle it; none was
  taken.
