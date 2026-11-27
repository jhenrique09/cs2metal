# What the host changes gained, 27 November 2026

## Status

Measured on 27 November 2026 on the owner's Apple M1 Max, the game build 26879581,
MoltenVK 1.4.2, a 2560 by 1440 display, the game's video settings as the owner has
them with CMAA2 in place of multisampling. It follows
[where the time goes](PERFORMANCE_AB_2026-11-27.md), whose measurements led to the
changes, and compares the host at commit `88d94ee`, before them, with the host at
commit `4e27d4d`, after: the vDSO, the guest's threads in the user-interactive
class, the performance cores only, the system call entry's fast handler, and
Vulkan commands recorded on the guest's side. Every boot was offline, with
`-insecure`, under the authorization in `AGENTS.md`. Not edited after the fact.

## Questions

1. Are the costs the samples showed gone?
2. Does the game present more frames?
3. How many Vulkan calls does the renderer make, and which?
4. Which step of making a pipeline takes the time of a stall?
5. What is slow when the game reports its event processing as slow?
6. Does macOS turn Game Mode on for the host in its bundle?

## Method

- **Samples**, as before: two one-second samples of client boot 90, a match with
  bots on `de_dust2`, 1,372 samples on a processor, 501 of the main thread. The
  samples before were taken at 1920 by 1080 and these at 2560 by 1440, so the
  shares are compared by kind of cost, not frame for frame.
- **Frames.** Five pairs of boots, the old host and the new one in turn, 110
  seconds each on `de_dust2` with nobody at the keyboard, and two boots of the new
  host with `CS2METAL_NO_VULKAN_RECORD=1`. The pace is the trace's, with
  `CS2METAL_VULKAN_TRACE=pacing`, over the last minute of each boot.
- **Processor time.** In two of the pairs, the process's processor time between
  the 45th and the 105th second (`ps`), divided by the presents of that time.
- **Calls.** The trace's counts by function name (client boot 87) and the
  recorder's own counts (client boot 89).
- **Pipelines.** `CS2METAL_SHADER_TRACE=1` on a map loaded for the first time,
  `de_nuke`, for two minutes (client boot 88): MoltenVK's own timings.
- **Events.** `CS2METAL_SDL_TRACE=1` with the Steam overlay inserted, the boot of
  the first record that had the most slow event processing (client boot 91).

## Results

### The costs the samples showed

| Share of processor time | Before | After |
| --- | --- | --- |
| Main thread: reading the clock | 13.2% | 1.5% |
| Main thread: host calls through thunks | 9.1% | 1.3% |
| All threads: GS switches under the renderer's Vulkan calls | 9.3% | 2.3% |
| All threads: `futex`, through glibc's `syscall` | 3.9% | 0.7% |
| All threads: reading the clock | 3.3% | 0.4% |
| All threads: MoltenVK, Metal, and the GPU driver | 17.4% | 13.5% |
| All threads: the job threads spinning before they sleep | 8.7% | 10.3% |

The main thread was on a processor in 79% of its samples, where it had been in
70%: almost all of that time is the game's own code now.

### Frames, and processor time

| Presents a second, last minute of each boot | Boots | Mean |
| --- | --- | --- |
| Before | 106, 109, 90, 91, 90 | 97 |
| After | 110, 97, 98, 101, 114 | 104 |
| After, without command recording | 94, 104 | 99 |

| Processor time | Before | After |
| --- | --- | --- |
| Processors busy, of ten | 4.11, 3.62 | 3.99, 3.59 |
| Processor time a present, all threads | 45.0 ms, 40.4 ms | 39.6 ms, 31.4 ms |
| Mean | 42.7 ms | 35.5 ms |

- **The game does a sixth less work a frame**: 35.5 ms of processor time a
  present against 42.7 ms, in two pairs of boots. (CONFIRMED for those boots)
- **It does not show as more frames.** 104 presents a second against 97 is inside
  the scatter of either side, which runs from 90 to 114. (not shown either way)
- **The GPU is why.** MoltenVK's own report of boot 88 puts the GPU's execution of
  a command buffer at 12 to 13 ms on average and a present at 7.5 ms, at 2560 by
  1440. What the host saves is processor time the frame was not waiting for.
  (INFERRED; no boot was run at a size where the GPU is idle)

### The renderer's Vulkan calls

8,200 to 9,000 calls a present, of 74 functions, in a match with bots:

| A present | Calls |
| --- | --- |
| `vkCmdBindVertexBuffers` | 1,017 |
| `vkAllocateDescriptorSets` | 1,017 |
| `vkUpdateDescriptorSetWithTemplate` | 1,016 |
| `vkCmdBindDescriptorSets` | 627 |
| `vkCmdBindPipeline` | 510 |
| `vkCmdBindIndexBuffer` | 452 |
| The three draw commands | 969 |
| Dynamic state: scissors, viewports, stencil, depth, culling, topology, blending | 1,982 |
| `vkCmdBeginRenderingKHR`, `vkCmdEndRenderingKHR`, `vkCmdPipelineBarrier2KHR` | 420 |
| Everything else | 179 |

Three quarters are `vkCmd*` commands. With them recorded on the guest's side, a
match recorded 5,100 to 8,900 commands a present without a crossing and replayed
them in 18 to 24 flushes; one to three a present could not be recorded as they
were. The first record's estimate of five thousand calls a frame was low by two
fifths.

### What a stall is made of

Two minutes on a map loaded for the first time: 1,515 Metal libraries and 555
pipeline states made, 20.6 s in all.

| Step | Count | Mean | Longest | Sum |
| --- | --- | --- | --- | --- |
| SPIR-V to Metal's shading language | 133 | 5.5 ms | 25 ms | 0.7 s |
| Reading a translated shader from the game's pipeline cache | 1,286 | 0.4 ms | 1.5 ms | 0.5 s |
| Metal's compiler: shading language to a library | 1,515 | 10.8 ms | 437 ms | 16.4 s |
| Metal: library functions to a pipeline state | 555 | 6.9 ms | 164 ms | 3.9 s |

- **The time is Metal's compiler.** Four fifths of it turns shading language into
  libraries, and most of those are quick: the shortest took 0.024 ms, the mark of
  one Metal had compiled before, the longest 437 ms.
- **The game's own pipeline cache already spares the translation**: nine shaders
  in ten came from it translated. MoltenVK's part is 6% of the time.
- **So nothing in the host shortens a stall.** The work is in a system service,
  and what it has compiled it keeps: playing a map is what makes the next match
  on it smooth.

### The slow event processing

With the overlay inserted and the trace on, five event pumps took 3 ms or more:
66, 7.4, 7.6, 4.6, and 4.2 ms, all in the four seconds in which the window came
up, and all of it inside host SDL, where it takes the application's events from
AppKit. The bridge's own look at the window was never part of one. The game
reported no slow event processing in that boot. The 7 to 18 ms of the online
match, with a player at the mouse, were not reproduced with nobody there. (OPEN)

### Game Mode

Not shown. The host was installed in its bundle and started both ways, offline:

- **From a shell** (`run.sh`), macOS's game policy daemon never names the process
  in its log.
- **Through a shortcut application**, as Launch Services starts one, the daemon
  follows the process from its start, under the shortcut's own identity, since
  the game is that process from the shortcut's first program to the host. The
  shortcut's `Info.plist` therefore matters as much as the host's, and both now
  ask for Game Mode.

No line of the log says that Game Mode was turned on or left off, and nothing
else here can tell: macOS decides, and shows a Game Mode item in the menu bar
while it is on. Two boots each way presented at the same pace, 10.4 to 13.1 ms
between presents, so starting through the shortcut costs nothing. The first
start of the host from its new place spent four minutes compiling Metal's
shaders again, once, as expected. (OPEN: the owner can see the menu bar item)

## Interpretation

- **The bridges no longer take what the first record measured.** Reading the
  clock, the futex calls, and the GS switches of Vulkan commands went from a
  quarter of the main thread's time and a sixth of all processor time to a few
  percent.
- **The frame rate at 2560 by 1440 did not move, and was not going to**: the GPU
  sets it there. The saving should show where the processor sets it: at a smaller
  size, on the low preset, with more players. That is not measured.
- **The stalls are Metal's compiler, and the host cannot shorten them.**
- **What is left of the host's own cost** is the quarter of Vulkan calls that
  still cross one by one, the descriptor sets ([future work](FUTURE_WORK.md)).

## Open

- A boot where the processor limits the frame, to see the saving as frames.
- A traced session with the owner playing, for the slow event processing:
  `CS2METAL_SDL_TRACE=1 ~/cs2-metal/bin/run.sh`, and the log.
- Whether the efficiency cores being left out helps or costs: it was not measured
  apart from the rest.
- An online match on the new host. Only the owner can play one.
- Whether macOS turns Game Mode on, and what it changes if it does.
