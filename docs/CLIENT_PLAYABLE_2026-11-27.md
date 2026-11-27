# The client, playable offline, 27 November 2026

## Status

Measured on 27 November 2026, client boots 47 to 56 of the day and four sessions the
owner ran and played, under the authorization in `AGENTS.md`, always with
`-insecure`. Follows [the client's first frames](CLIENT_FIRST_FRAMES_2026-11-27.md).
The machine is an Apple M1 Max on macOS 27 with a 2560 by 1440 display, the driver
MoltenVK from Vulkan SDK 1.4.363.1, the game build 26879581. Not edited after the
fact.

## Question

Can the owner play a practice match: does input work in a match, does sound play,
and how steady are the frames?

## Method

The owner played: the menu, the settings, practice matches on `de_dust2`, the
game's console. The agent ran boots with `+map de_dust2` and read three traces.
`CS2METAL_SDL_TRACE` counts the events the game takes and names its input calls with
the guest code that made them. `CS2METAL_VULKAN_TRACE` reports the pacing of
presents every ten seconds. `CS2METAL_STACKS` and the fault report were not needed.

## Results

- **The owner plays a practice match.** The menu, the settings, a match with bots,
  keyboard and mouse look all work, with sound. (CONFIRMED by the owner)
- **The game finds its window by an X11 window number.** Its tier-0 module reads
  the window property `SDL.window.x11.window` as the window's handle, and its input
  system finds the window again through that handle before each call for relative
  mouse mode or text input. Host SDL's video driver is Cocoa and sets no such
  property, so those calls arrived with a null window and failed: the pointer
  stayed visible and free in a match and the console took no text. With the
  property set by the bridge (commit `f658733`), boot 54 showed relative mouse mode
  switching on in the match and text input events reaching the game while the
  owner typed.
- **`SteamAPI_IsSteamRunning` answers for the macOS client.** It reads a process id
  from `~/.steam/steam.pid`; the bridge answers that path with the id of the
  running macOS Steam client (commit `118cd54`).
- **Frame pacing, by the trace.** In the menu: a median of 14.0 ms between
  presents, the 99th percentile at 15 to 16 ms, at most one slow present in ten
  seconds. In a match at 1920 by 1080: a median of 12.6 ms, with the slow presents
  (over twice the median) falling from 35 in ten seconds at the start to one. At
  2560 by 1440 with four-sample antialiasing, after the owner chose the high
  preset: a median of 15.4 to 20.7 ms, 0 to 5 slow presents in ten seconds, with
  isolated presents of 400 to 980 ms. The game's own spike reports put those in
  its client rendering.
- **The launch options forums suggest change nothing measurable.** The same
  two-minute boot with `-nojoy -novid -console -low -fullscreen` had a median of
  15.1 to 21.8 ms and 0 to 4 slow presents in ten seconds, inside the variation
  between boots.
- **Changing the video preset stalled the game once**, for 19 seconds by the
  renderer's own message about waiting for the GPU, and then went on.

## Open

- The small stutters: whether the isolated long presents end once every shader has
  been compiled once is not measured over a long session.
- In one session the menu had no music until the owner changed the video preset;
  in another the sound and the input stopped for a while and came back. Neither
  was traced or reproduced. The first session logged that the audio device was
  starved of 740 samples at its start.
- One boot of 23 aborted on AppKit's exception for events taken off the main
  thread ([first frames](CLIENT_FIRST_FRAMES_2026-11-27.md)); not seen again.
- The game's video settings offer no exclusive full screen. The window is already
  a full-screen SDL window, by its flags in the trace. Whether the Linux build
  offers that mode on Linux is not checked.
- A session's log held 3,423 Pango assertion messages about text ranges, the last
  of them when the owner tried the console; none in boot 54.
- Game controllers, the microphone, matchmaking, and secure servers are not
  tried. The last two stay owner actions and outside `-insecure`.

## Interpretation

- **M5's gate is met**: a window opens, input works, and sound plays. (CONFIRMED)
- **M8 is in reach but not passed**: its gate is thirty minutes of a local bot
  match without a crash, with the integrity check before and after. The sessions
  so far were shorter and not timed. (OPEN)
- **Performance is the driver's and the translator's, as far as measured.** Nothing
  in these traces points at the bridges for the steady frame time; the first-use
  stutters match shader compilation, which is inferred, not measured. (INFERRED)
