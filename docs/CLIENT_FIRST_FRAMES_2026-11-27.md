# The client's first frames, 27 November 2026

## Status

Measured on 27 November 2026, client boots 33 to 46 of the day, under the
authorization in `AGENTS.md`, with `-insecure`, each under an 85-second timeout. The
owner watched the screen and photographed it. **Corrects the interpretation of
[what the client's window shows](CLIENT_RENDERING_2026-11-27.md)**: the renderer did
not stop because its window was not in front. Not edited after the fact.

## Question

Why does the renderer stop presenting after one or two frames, and does the client
render once it does not?

## Method

- The system call entry was given a standard frame record, so that host samples of
  a thread inside a system call continue into the guest code that made it.
- `CS2METAL_FUTEX_TRACE` reported the waits in progress and the most woken words.
- A report of fatal signals, and `CS2METAL_STACKS`, wrote the instruction, the
  registers, and the return addresses of faulting and of running threads.
- `CS2METAL_SDL_TRACE` counted the audio callbacks, what the game queues, and what
  the device is given. `CS2METAL_VULKAN_TRACE` counted presents, as before.
- Addresses were read as guest modules and offsets with `CS2METAL_MAPS=1`.

## Results

- **`gettimeofday` left half of the guest's microseconds unwritten.** System call
  96 was answered by the host's own call on the guest's pointer; the host's
  `struct timeval` holds `tv_usec` in 32 bits and Linux's in 64. The game's tier-0
  semaphores build absolute timeouts from that value, glibc's
  `pthread_cond_timedwait` refused them, and timed waits returned at once: the
  trace counted 8,127,788 wakes of one mutex word in ten seconds. With the value
  whole (commit `e364127`), boot 36 passed 128 presents in its first 40 seconds
  and the owner photographed the main menu, drawn by the Vulkan renderer.
- **The audio callback was called too early and without its lock.** Called before
  the game resumed its device, the game's callback divided by a frame size that
  was still zero (SIGFPE in boots 34, 37, and 38). Called without the stream
  locked, its own lock was never released and SDL's device thread waited on the
  stream forever. Both are fixed in commit `7959198`; the callback then ran more
  than 2,000 times a boot.
- **MoltenVK reads through null descriptor sets.** The renderer binds ranges of
  sets with `VK_NULL_HANDLE` among them, four of five bound in the first such
  call. The thread that submits frames stopped at one instruction inside
  MoltenVK's `vkCmdBindDescriptorSets` encoding, with the same registers thirty
  seconds apart, and the main thread waited for it. With the null sets left out
  (commit `13ba9d4`), boot 42 passed 2,048 presents in 85 seconds, and the owner
  started a practice match from the menu: the photograph shows the map, the
  weapon, and the HUD, with the game's own counter at about 96 frames per second
  on average and a longest frame of 333.8 ms.
- **The owner heard no sound** in boots 36 and 42. In boot 46 the game opened the
  default playback device, resumed it once, never cleared its stream, and queued
  samples that were not silent; the mix SDL handed the device peaked at 0.41 of
  full scale. Where the sound is lost after that, or whether the owner was
  listening to another output, is not established.
- **Boot 39 aborted** with AppKit's exception for events taken off the main
  thread. It did not repeat in boots 40 to 46, and which SDL call made it is not
  established.
- **The match's colors look pale in the photograph.** Whether that is the rendered
  image or the photograph is not established.

## Interpretation

- **F3's gate is met, and the plan's second gate with it**: the game presents its
  startup sequence, its main menu, and a match in its own window, through the
  SDL and Vulkan bridges on MoltenVK. (CONFIRMED, by the owner's photographs and
  the present counts)
- **Mouse input reaches the game**: the owner started the match from the menu.
  Keyboard input and mouse look inside the match are not measured. (CONFIRMED for
  the menu)
- **The earlier reading was wrong.** The stall did not depend on the window's
  focus; it was the clock structure, then the null descriptor sets. (CONFIRMED)
- **Sound is the next thing to settle**, with the owner at the machine: a test
  tone through the same SDL, and the macOS output device. (OPEN)
