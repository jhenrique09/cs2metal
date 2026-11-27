# Where the time goes, and what was tried, 27 November 2026

## Status

Measured on 27 November 2026 on the owner's Apple M1 Max (eight performance cores,
two efficiency cores, macOS 27.2.1), the host at commit `88d94ee`, the game build
26879581, MoltenVK 1.4.2 from Vulkan SDK 1.4.363.1, a 2560 by 1440 display. Every
boot was offline, with `-insecure`, under the authorization in `AGENTS.md`. The
online match is the owner's own session, read afterwards from the game's console
log. The host changes that followed from this are measured in a later record.
Not edited after the fact.

## Questions

1. In a match, which thread limits the frame, and what does it spend its time on?
2. What do the bridges cost: the GS switches of system calls and of Vulkan calls?
3. What are the stalls of an online match?
4. Do the settings that could be tried without a change of code help: the
   anti-aliasing, three MoltenVK and engine options, the Steam overlay?

## Method

- **Samples.** Five one-second host samples (`sample`) of matches with bots on
  `de_dust2` at 1920 by 1080, client boots 43 to 46 and 70 of the day, each named
  against the boot's own module map. A sample is a call tree per thread; each
  sample was given to the innermost frame that names a module, and a thread
  blocked in a kernel wait was counted as waiting. Together they hold 955
  samples on a processor, 313 of them of the main thread.
- **The cost of a GS switch.** A switch is a system call that Rosetta's runtime
  answers. Its samples end in one runtime routine, the same under
  `clock_gettime`, under glibc's `syscall`, and under the renderer's functions
  that call Vulkan: the host thunk leaves no frame of its own, so the switch
  shows directly under the guest code that made the call.
- **The online match.** The game writes a report of each frame it finds too long,
  with the time by part of the frame. The owner's session of 16:43 to 16:48 on
  `de_mirage`, a map not played before, has 28 of them.
- **The A/B boots.** `+map de_dust2`, 110 seconds, nobody at the keyboard, the
  installed host. `CS2METAL_VULKAN_TRACE=1` reports the pacing of presents every
  ten seconds; the figures below are the last six periods, a minute in the map.

## Results

### The main thread limits the frame

The main thread was on a processor in 70% of its samples. In the sample of boot
70 no other thread was on one in more than a fifth of its own.

| Of the main thread's processor time | Share |
| --- | --- |
| The game's own modules (client, server, engine, user interface, network, animation, physics) | 78% |
| `clock_gettime`, as a system call | 13% |
| Host calls through thunks, four fifths of them SDL's event pump | 9% |

It waited in 30% of its samples, half of that for jobs it had handed to the job
threads. In a match with bots the server runs on the same thread: 11% of its
processor time, which an online match does not have.

### What the bridges cost

| Of the processor time of all threads | Share |
| --- | --- |
| MoltenVK, Metal, and the GPU driver | 17.4% |
| GS switches under the renderer's Vulkan calls | 9.3% |
| The job threads spinning before they sleep | 8.7% |
| `futex`, through glibc's `syscall` | 3.9% |
| `clock_gettime` | 3.3% |

- **A Vulkan call's two GS switches cost half of what the driver does with the
  call.** At 520 ns a pair ([segment probe](ROSETTA_SEGMENT_PROBE_2026-11-26.md))
  and about a hundred frames a second, the samples put the switches at 2 to 3 ms
  of processor time a frame, spread over the render threads, which is on the
  order of five thousand Vulkan calls a frame. (INFERRED from the sample rate;
  the calls were not counted.)
- **Reading the clock is a system call, and the game reads it constantly.** There
  was no vDSO, so each read took the system call entry and two GS switches.
- **The job threads spin.** The engine's job pool spins with `pause` before a
  thread sleeps; under Rosetta that shows as a runtime routine. It is the game's
  own design, and nothing the host does.

### The stalls of an online match are first-use pipelines

| Measure | Value |
| --- | --- |
| Frames the game reported as too long | 28, in four minutes |
| Their sum, and the longest | 12.0 s, 1.1 s |
| In the first minute | 16 |
| With client rendering as their largest part | 24, 9.9 s of the 12.0 |
| Times the client then jumped its tick to the server's | 2 |
| Times the game reported its SDL event processing as slow | 15, of 7 to 18 ms |

The renderer creates a pipeline when it first draws with it, and the frame waits.
Neither driver for macOS offers graphics pipeline libraries, which is what spares
the Linux drivers that wait
([driver measurement](VULKAN_DRIVER_MEASUREMENT_2026-11-27.md)). After a stall of
half a second the client is far behind the server and jumps: the player sees the
world skip. Both caches keep what was compiled: the game's own pipeline cache held
94 MB that evening and Metal's 43 MB. A map played once does not stall the same
way again. (INFERRED: the second match on a map was not measured.)

### The settings

| Boot | What differs | Presents a second | Median | 99th percentile | Presents over twice the median |
| --- | --- | --- | --- | --- | --- |
| a0 | The owner's settings: multisampling 2x | 81 | 12.4 ms | 16.2 ms | 11 |
| b0 | CMAA2 in its place, first boot after the change | 62 | 14.7 ms | 26.4 ms | 51 |
| b1 | CMAA2, second boot | 104 | 10.5 ms | 13.9 ms | 6 |
| | `MVK_CONFIG_PREFILL_METAL_COMMAND_BUFFERS=1` | 2 | 1013 ms | | |
| | `MVK_CONFIG_SYNCHRONOUS_QUEUE_SUBMITS=0` | 87 | 12.3 ms | 17.5 ms | 17 |
| | `-threads 8` | 100 | 10.8 ms | 14.9 ms | 8 |
| | The Steam overlay inserted, as `run.sh` does | 80 | 12.9 ms | 22.4 ms | 36 |

- **Prefilled Metal command buffers do not work with this renderer**: one present
  a second. (CONFIRMED)
- **Two boots of the same settings differ by as much as any variant differs from
  them.** b0 and b1 are the same settings: the first compiled the pipelines of
  the new render targets, and bots do not repeat a match. One boot each cannot
  rank the rest.
- **CMAA2 in place of multisampling 2x: 104 against 81 presents a second**, one
  settled boot each. The direction fits the cause, since multisampling doubles
  the size of every render target of a 2560 by 1440 window; the size of the gain
  is one pair of boots. (INFERRED)
- **Asynchronous queue submits and eight threads show no gain.** (not shown
  either way)
- **The overlay boot had more slow presents and more slow event processing than
  any other**: 36 presents over twice the median, and five reports of slow SDL
  event processing where the others had none to two. One boot. (OPEN)

## Interpretation

- **The frame is limited by the game's own code on one thread, translated by
  Rosetta.** No bridge change raises that ceiling; what the bridges can do is
  give that thread back what they take from it: the 13% of clock reads and the
  share of the 9% that is not the event pump itself.
- **The Vulkan bridge is the largest single cost the host adds**: 9.3% of all
  processor time, on the render threads.
- **The stalls are a property of the driver, not of the host.** Short of
  pipelines compiled before the match, the remedy is to have played the map.
- **At 2560 by 1440 the anti-aliasing mode is the setting that matters.**

## Open

- What the host changes this led to gain: a later record.
- A repeatable scene. Without one, differences under a fifth cannot be told from
  the scatter between boots.
- What the slow SDL event processing is, and whether the overlay causes it.
- Whether the stalls do end on the second match on a map.
- No sample was taken at 2560 by 1440, nor online.
