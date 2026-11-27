# The first client boots, 27 November 2026

## Status

Measured on 27 November 2026 on build 26879581 of the CS2 Linux build, for roadmap
steps E1, F2, and F3. Client boots 1 to 8 of the day, run by the agent under the
authorization in `AGENTS.md`: through `cs2metal run`, with `-insecure`, under
timeouts of 60 to 150 seconds, with the owner's macOS Steam client signed in. The
owner watched the screen. The logs stay in the ignored `runs/` directory and the
game's own console log in the ignored overlay. This document cites only counts,
system call numbers, and public interface names. Not edited after the fact.

## Question

How far does the client, with a window and the owner's Steam client, boot under
cs2metal, through the [SDL bridge](SDL_BRIDGE.md) and the
[Vulkan bridge](VULKAN_BRIDGE.md)?

## Method

```bash
target/x86_64-apple-darwin/release/cs2metal run \
  --game-dir ~/cs2metal-kit/cs2-linux --runtime-dir ~/cs2metal-kit/sniper/files \
  -- -insecure -condebug
```

Each boot first checked all 3,131 files of the installation against the baseline,
with no difference. `-condebug` makes the game write its console log, which goes
to the overlay.

## Results

| Boots | What stopped the client | Fixed by |
| --- | --- | --- |
| 1 to 4 | A record lock on its lock file failed (`fcntl` `F_SETLK` unanswered), then nothing more after loading SDL | translating record locks (commit `bce901d`) |
| 4, 5 | After the renderer module loaded, the game's message that Vulkan 1.2 could not be initialized, shown in a macOS dialog by the host's SDL | the Vulkan bridge (commit `078c1f3`) |
| 6, 7 | `vkCreateInstance` and `vkCreateDevice` succeeded on MoltenVK, on the Apple M1 Max. The game opened its window and **presented its startup screen**, which the owner photographed. Then a fatal error: the CPU frequency could not be determined | synthesized `/proc` and `/sys` files (commit `cf7c008`) |
| 8 | Nothing: **the game reached its main menu state** about 15 seconds after start, signed in to Steam, measured its ping to 24 Steam relays, and ran until the 150-second timeout | |

The renderer asked for five instance extensions, all present on the host, and 21
device extensions, all present on MoltenVK's device.

Boot 8 also showed what is still missing, none of it fatal:

- The game's audio stream callback is not bridged yet, so there is no sound.
- Writing two user configuration files failed. The same values were saved to Steam
  Cloud.
- Four calls were unanswered: `flock` (73), `utimes` (235), `semget` (64), and
  `semtimedop` (220). They are answered since commit `071dc96`.
- Steam's own check for a running client looks for Linux Steam's files and does
  not find them. The game's Steam API library loads the bridge anyway.

## Interpretation

- **The goal of the roadmap is met: the game presents its startup screen in its
  own window.** It runs the unmodified Linux build's renderer on MoltenVK, through
  SDL 3 for macOS, under Rosetta 2. (CONFIRMED by the owner's photograph and the
  game's log)
- **The game reaches its main menu state.** The console log shows the UI moving to
  the main menu. Whether the menu renders correctly, the roadmap's second gate, is
  for the owner to see. (CONFIRMED in the log; the picture is to be confirmed)
- **The step before last was the kernel's view of the machine.** The engine reads
  its CPU frequency and topology from `/proc` and `/sys`. The synthesized
  frequency is the time stamp counter's rate under Rosetta 2, 1,000 MHz, measured,
  so time computed from `rdtsc` agrees with the clock. (CONFIRMED)
- **What comes next is playability:** sound (the audio callback on a guest thread),
  input in the window (E1's gate), and the configuration writes. (INFERRED)
