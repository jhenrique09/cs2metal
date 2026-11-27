# The SDL bridge

## Status

Design and first implementation for roadmap step E1, 27 November 2026. Describes
`guest/shims/libSDL3.so.0.c`, `tools/sdl-shim-names.sh`, and
`crates/cs2metal-linux/src/sdl.rs`.

## What the game needs from SDL

The Linux build ships SDL 3 as `game/bin/linuxsteamrt64/libSDL3.so.0`, a
development build of SDL 3.5.0. The engine's tier-0 module loads it with `dlopen`
and resolves about 140 functions by name; the Panorama UI module links about 30
more, with the symbol version `SDL3_0.0.0`. Together they cover the window, the
display modes, keyboard, mouse, gamepads and haptics, the clipboard, message
boxes, audio streams, and the Vulkan surface. (CONFIRMED from the modules' dynamic
symbols and strings, read locally, 27 November 2026; the names are SDL's public
API.)

## Design

Like the [Steam bridge](STEAM_BRIDGE_DESIGN.md), the bridge replaces one library
at the boundary and leaves the game's own code alone:

1. cs2metal serves its shim wherever the guest opens a `libSDL3.so.0`. The game's
   file is never read or changed.
2. The shim exports every function the game's SDL exports, with the same version.
   Its list is generated from the game's own `libSDL3.so.0` by
   `tools/sdl-shim-names.sh`, at build time, so it always matches the game's
   version.
3. The shim's constructor makes the private system call `0x43530002`. The host
   loads an x86-64 SDL 3 for macOS into its own domain and fills the shim's table,
   by name, with host thunks into it.
4. SDL 3's ABI is the same on x86-64 Linux and macOS (both System V), so structures
   such as `SDL_Event` and `SDL_AudioSpec` cross the bridge unchanged, and so do
   variadic calls, since the host thunk keeps `rax` and the stack arguments.

The host SDL is, by default, the universal SDL 3 framework of the newest Vulkan
SDK (3.4.16 in SDK 1.4.363.1); `CS2METAL_SDL3` names another. It lacks 32 of the
game's 1,302 functions, none of which the game resolves: extended reality, GDK,
notifications, and a few string helpers. Those entries point at a stub that
names the call.

### Wrapped functions

| Function | Why it is wrapped |
| --- | --- |
| `SDL_SetLogOutputFunction` | The game's log function is guest code. Host SDL calls a host wrapper, which runs the game's function through a guest thunk on a guest thread, and writes to standard error on any other thread |
| `SDL_AddEventWatch`, `SDL_RemoveEventWatch` | The watch is guest code, called the same way; events pushed on host-only threads skip it |
| `SDL_SetClipboardData` | Its callbacks are guest code, called on the guest's main thread |
| `SDL_OpenAudioDeviceStream`, `SDL_DestroyAudioStream` | A stream callback would run on host SDL's audio thread, which runs no guest thread, and running it on another thread while SDL holds the stream's lock would deadlock. The stream opens without it; a guest thread the shim starts asks the host for each call to make, and the host hands it one whenever a playback stream's queue falls below 40 ms, or a recording stream has data, as SDL's device thread would. Two things SDL guarantees a callback are kept. The device starts paused, and no call is made until the game resumes it: the game finishes setting up its side in between, and its callback divided by a still-zero frame size when called earlier. The stream is locked on the callback thread around each call: the game's callback unlocks the stream while it mixes and locks it again before it returns, so without the lock taken first, the game's own lock was never released and SDL's device thread waited on it forever, in silence. `CS2METAL_SDL_TRACE=1` logs the stream's format and device, counts the calls, and counts what the game does with the stream: the bytes it queues with their peak sample, its clears, flushes, pauses, and resumes, the peak of the mix SDL hands the device, through a postmix callback, and the times the pump found a playing stream's queue empty. For input it counts the events the game takes by type, names the game's calls for relative mouse mode, mouse grab, text input, the cursor, and hints, with the guest code that made them, and names each change of the keyboard focus, the mouse focus, and the window's flags |
| `SDL_MinimizeWindow` | Not done. The game minimizes its window when it loses the focus, and a process started from a terminal starts without it; on macOS a minimized window has no drawable, so the window went to the Dock at once and stayed there. `SDL_VIDEO_MINIMIZE_ON_FOCUS_LOSS` is also set to 0 in the host environment |
| `SDL_CreateWindowWithProperties` | The window is given the number property `SDL.window.x11.window`, holding its SDL window id. The game keeps its windows by that operating system handle, and its input system finds the window again through it before asking for relative mouse mode or text input. Host SDL's video driver is Cocoa and sets no such property, so the handle read as zero, the window as null, and those calls failed: the pointer stayed visible and free in a match and the console took no text. Nothing else in the game reads the number. The window is also raised once made, since the process does not come to the front by itself |
| `SDL_SetWindowSize`, `SDL_SetWindowPosition`, `SDL_SetWindowBordered`, `SDL_SetWindowFullscreen`, `SDL_SetWindowFullscreenMode`, `SDL_SyncWindow`, `SDL_RestoreWindow`, `SDL_DestroyWindow`, `SDL_PeepEvents` | The game's video settings offer a window and a "full screen window". For the second it makes a borderless window as large as the display, or asks SDL for full screen, and on Linux the window manager does the rest. On macOS the Dock and the menu bar stayed in front of the borderless window. After each of these calls, and a few times a second where the game takes its events, the bridge looks at the window: while it is shown and either held full screen by SDL or borderless and as large as its display, the application's presentation options hide the Dock and the menu bar, and otherwise they are the default again. `SDL_VIDEO_MAC_FULLSCREEN_SPACES` is also set to 0 in the host environment: in a full-screen Space, changing the display mode left the window with a title bar while SDL reported it full screen |
| `SDL_IOFromFile` | A Linux path: it goes through the installation overlay |
| `SDL_Vulkan_LoadLibrary` | The game would name its Linux loader. Host SDL loads the macOS Vulkan loader, by default the newest Vulkan SDK's, which finds MoltenVK through the SDK's driver manifest unless `VK_DRIVER_FILES` is set |

A guest callback runs through the guest thunk (`Bridge::guest`): GS switches to
the guest's control block for the call, and back. Host code finds the thread's
host block through a pthread TSD slot. A host thread that never ran guest code has
no guest state, so the wrappers keep guest callbacks off such threads.

### Quitting

SIGTERM and SIGINT (Ctrl-C in the terminal that started the game) used to end the
process where it stood. They now set a flag, and where the game next takes its
events the bridge pushes SDL's quit event, the one closing the window sends, so the
game shuts itself down. The default action is put back at the first signal, so a
second one ends a game that does not quit. A process ended in the middle of its work
leaves what it was writing as it was; after test boots ended that way, the next
start spent two minutes compiling its Metal shaders again (INFERRED: the order of
events fits, the cause is not isolated).

### The main thread

macOS requires windows and events on the process's main thread. cs2metal starts
the guest on the thread `cs2metal run` was started on, which is the process's
main thread, so the game's own main thread is where host SDL creates its window
and pumps its events.

## What remains

- Other callbacks on host-only threads, such as SDL timers, could use the same guest
  callback thread.
- `SDL_Vulkan_CreateSurface` takes a `VkInstance`, which must be the host's, so the
  game's Vulkan calls must reach the macOS loader too. That is step F2.
