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
| `SDL_OpenAudioDeviceStream` | A stream callback would run on host SDL's audio thread, which runs no guest thread. The game feeds its streams with `SDL_PutAudioStreamData`, so a callback is not bridged yet and is named if one comes |
| `SDL_IOFromFile` | A Linux path: it goes through the installation overlay |
| `SDL_Vulkan_LoadLibrary` | The game would name its Linux loader. Host SDL loads the macOS Vulkan loader, by default the newest Vulkan SDK's, which finds MoltenVK through the SDK's driver manifest unless `VK_DRIVER_FILES` is set |

A guest callback runs through the guest thunk (`Bridge::guest`): GS switches to
the guest's control block for the call, and back. Host code finds the thread's
host block through a pthread TSD slot. A host thread that never ran guest code has
no guest state, so the wrappers keep guest callbacks off such threads.

### The main thread

macOS requires windows and events on the process's main thread. cs2metal starts
the guest on the thread `cs2metal run` was started on, which is the process's
main thread, so the game's own main thread is where host SDL creates its window
and pumps its events.

## What remains

- Callbacks on host-only threads (an audio stream callback, SDL timers): they need a
  guest thread to run on, such as a guest callback thread that runs them on
  request.
- `SDL_Vulkan_CreateSurface` takes a `VkInstance`, which must be the host's, so the
  game's Vulkan calls must reach the macOS loader too. That is step F2.
