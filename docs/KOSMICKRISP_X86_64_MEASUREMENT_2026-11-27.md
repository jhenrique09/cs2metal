# KosmicKrisp built for x86-64, measured in the game, 27 November 2026

## Status

Measured on 27 November 2026 on the owner's Apple M1 Max, macOS 27.2.1, the game
build 26879581, at 2560 by 1440 with the owner's video settings. KosmicKrisp was
built from upstream Mesa at commit `c51e70c3ff` with one local change, and compared
with MoltenVK 1.4.2 from LunarG's SDK 1.4.363.1. Every boot was offline, with
`-insecure`, under the authorization in `AGENTS.md`. It answers the entry
"KosmicKrisp built from Mesa source for x86-64" of [future work](FUTURE_WORK.md),
and with it the first condition of the
[native driver process study](NATIVE_VULKAN_PROCESS_STUDY_2026-11-27.md). Not
edited after the fact.

## Questions

1. Does KosmicKrisp build for x86-64 and load in the host under Rosetta 2?
2. Does the game run on it?
3. Does it present more frames than MoltenVK?
4. Does `MVK_CONFIG_VK_SEMAPHORE_SUPPORT_STYLE=2` help MoltenVK, as one pair of
   boots of the same day suggested?

## Method

- **Build.** `spikes/kosmickrisp-x86_64/build.sh` (its README tells how to use it):
  Mesa's compiler tools natively, then the driver cross-compiled for x86-64, the
  two stages of the third-party build at `shadexternals/mesa-kosmickrisp`, whose
  scripts were read and whose binaries were not used. Running the script again
  gave the same library, byte for byte.
- **Without the game.** `vulkaninfo` under `arch -x86_64`, `cs2metal vulkan
  compare` against the renderer module, the
  presentation probe of `spikes/vulkan-present` under Rosetta 2, and a
  small Metal program for Metal 4 and the visibility buffer's limit.
- **With the game.** Boots on `de_dust2` with bots and nobody at the keyboard. Then
  ten alternating boots of 110 seconds each: MoltenVK, KosmicKrisp, MoltenVK with
  MTLEvent semaphores, four rounds, KosmicKrisp in the first two. Presents a second
  over the last minute of each boot, from `CS2METAL_VULKAN_TRACE=pacing`, and the
  process's processor time from its 45th to its 105th second.
- **The game's pipeline cache.** The game keeps one, and replaced its 93 MB of
  MoltenVK entries with 172 KB of KosmicKrisp's at the first boot on the other
  driver. Each driver's copy was put back before each of its boots.

## Results

### Without the game

| Check | Result |
| --- | --- |
| Metal 4 in an x86-64 process | Offered: the device reports the family and makes a Metal 4 command queue and allocator |
| `vulkaninfo`, x86-64 | KosmicKrisp, Mesa 27.0.0-devel, Vulkan 1.4.368, conformance 1.4.6.2, the M1 Max |
| Device extensions | 145, as the SDK's arm64 build: 35 of the renderer's 68, against MoltenVK's 33 |
| Of the 21 the game enables on MoltenVK | 20; `VK_KHR_fragment_shader_barycentric` is missing |
| Presentation probe under Rosetta 2 | Presented: 300 frames at the display's rate, first frame 295 ms (MoltenVK 60 ms) |

On KosmicKrisp the game enables `VK_EXT_multi_draw` and
`VK_EXT_conditional_rendering`, which MoltenVK lacks, and does without the
barycentric extension. (CONFIRMED)

### Two failures, and their fixes

1. **The host ended on an AppKit exception.** At the first boot, with every shader
   new, presents came two seconds apart, and the game pumped SDL's events from a
   thread other than the main one while it waited for one. AppKit allows that only
   on the main thread and ended the process. The SDL bridge now hands such a
   thread only the events already queued ([SDL bridge](SDL_BRIDGE.md)). The cause
   is the host's, not the driver's; MoltenVK never made a present that late.
2. **The game stopped at its third occlusion query pool.** It makes pools of
   16,384 occlusion queries, and KosmicKrisp gives every query of a pool an entry
   of one device-wide table when the pool is made. The table has 32,768 entries,
   so the third pool failed with `VK_ERROR_OUT_OF_DEVICE_MEMORY`, the renderer
   treated that as fatal, and the picture froze. The table cannot grow: a probe
   showed Metal counting into a visibility buffer only below 256 KiB, 32,768
   eight-byte results, and Metal's validation layer stops at the next offset. The
   local change, `lazy-occlusion.patch`, gives a query its entry when it is first
   begun. Afterwards the game made five pools and used 42 entries. (CONFIRMED)

After both, every boot on KosmicKrisp ended normally at SIGTERM. The first ones
stopped for seconds at a time while Metal's compiler service built shaders, one
at a time, as MoltenVK's first start did.

### Frames

| Presents a second, last minute | Boots | Mean |
| --- | --- | --- |
| MoltenVK | 76.0, 96.7, 91.1, 88.3 | 88.0 |
| KosmicKrisp, x86-64 | 55.3, 43.0 | 49.2 |
| MoltenVK, MTLEvent semaphores | 79.6, 97.1, 73.0, 77.5 | 81.8 |

| Processor time a present, all threads | Mean |
| --- | --- |
| MoltenVK | 38.1 ms |
| KosmicKrisp | 45.5 ms |
| MoltenVK, MTLEvent semaphores | 43.2 ms |

- **KosmicKrisp presents 44% fewer frames.** Its faster boot is below MoltenVK's
  slowest. (CONFIRMED for this scene and size)
- **The GPU does more a frame on it.** In a boot after its shaders were built the
  GPU was busy 71 to 74% at 43 frames a second. On MoltenVK, in the owner's match
  of the same day at 1920 by 1080, it was busy 84 to 92% at about 97. One thread of
  the game was busy 97%. (INFERRED from utilization at two sizes; no GPU timings
  were taken on KosmicKrisp)
- **MTLEvent semaphores do not help.** Their mean is below the default and their
  processor time a present above it. The single pair of the same day, 93.9 against
  101.9, was inside the scatter that this series shows: 73 to 97 for one
  configuration. (not shown either way; no gain)

## Interpretation

- **KosmicKrisp works in the host**, with one fix in Mesa, and the host is the
  sturdier for the other.
- **It is not faster here.** The gap is mostly GPU work, which no native driver
  process would remove: Rosetta translates the processor's code, not the GPU's.
  The native process study's case for a second driver rested on KosmicKrisp being
  better, so it stays a study, and a less likely one.
- **MoltenVK's defaults stay.**
- **The occlusion query fix belongs upstream.** It is a KosmicKrisp limit that any
  application making large pools meets, Rosetta or not. Reporting it is the
  owner's call.

## Open

- Whether KosmicKrisp's gap closes with its newer work: the measured commit is one
  day's.
- Whether occlusion results stay right over a long session: entries are kept until
  their pool is destroyed, and a game that begins every query of a pool would fill
  the table again; the change then shares one entry among the rest, which gives
  wrong counts but no failure.
