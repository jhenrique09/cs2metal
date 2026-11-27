# What the client's window shows, 27 November 2026

## Status

Measured on 27 November 2026, client boots 16 to 26 of the day, under the
authorization in `AGENTS.md`, with `-insecure`. **Corrects the
[first client boots](FIRST_CLIENT_BOOT_2026-11-27.md) record**, which read the Valve
logo as the renderer's first frame. Not edited after the fact.

## Question

Which of the client's windows shows what, and does its Vulkan renderer present
frames?

## Method

Window captures of the game's windows by their window numbers (`screencapture -l`),
while the owner watched the screen remotely. `CS2METAL_VULKAN_TRACE=1` logged the
swapchain, acquisitions, presents, submissions, and fence waits. Host samples of the
process were read as guest modules and offsets with `CS2METAL_MAPS=1`.

## Results

- **The game has two full-screen windows.** An untitled one shows the Valve logo.
  The game's tier-0 module draws it in software with SDL's surface functions,
  `SDL_LoadBMP_IO` and `SDL_BlitSurfaceScaled`. The second, titled
  "Counter-Strike 2", is the Vulkan renderer's, and it stayed black.
- **The renderer's swapchain works, then stops.** It is created at 2560 by 1440
  (`VK_FORMAT_B8G8R8A8_SRGB`, immediate present mode, three images). The renderer
  acquired four images, submitted work whose fences signalled, and presented
  twice, all successfully. Then it presented nothing more in the rest of each
  boot, while the game reached its main menu state and its main loop waited on
  SDL events.
- **The window never had the focus.** Started from a terminal while the owner used
  other applications remotely, the game did not come to the front. When it lost
  the focus, its request to minimize the window left it minimized in the Dock, and
  the window could not be restored. That is fixed in commit `f73c598`. Hiding
  `VK_KHR_present_wait` and `VK_KHR_present_id`, and launching with
  `+engine_no_focus_sleep 0`, did not change the two presents.

## Interpretation

- **F3's gate is not met yet.** The logo of the earlier record is SDL's software
  splash, not a Vulkan frame. What it does show is the window and SDL bridges at
  work, and the colors of a BMP crossing them intact. (CONFIRMED)
- **The renderer stops presenting while its window is not in front.** The likeliest
  reason is the engine throttling rendering while macOS reports its window
  occluded or unfocused. The next boot is to be watched with the window clicked to
  the front. (INFERRED)
