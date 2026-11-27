# Study: the Vulkan driver in a native arm64 process, 27 November 2026

## Status

A study, written on 27 November 2026 at the owner's request. Nothing here is built
and nothing is decided: it describes what the route would be, what exists to build
it from, what stands in its way, and what would have to be measured before it is
worth starting. Facts from other projects are **REPORTED** from their own pages,
read that day and listed under Sources; none was run here. Not edited after the
fact.

## Question

The game's Vulkan calls are answered inside the host process by MoltenVK, which is
x86-64 code there and so runs translated by Rosetta 2, like the game. Could a
second process, native arm64, run the Vulkan driver instead, with the host only
passing the calls on?

## Why ask

Three measurements of 27 November 2026, from five samples of a match with bots
([performance measurements](PERFORMANCE_AB_2026-11-27.md)):

- MoltenVK, Metal, and the GPU driver take 17% of the processor time of all the
  game's threads, all of it translated code.
- The GS switches around the renderer's Vulkan calls took 9% more, before
  commands were recorded on the guest's side of the bridge.
- MoltenVK is the only driver that loads in an x86-64 process. KosmicKrisp, the
  other one in the Vulkan SDK, is arm64 only
  ([driver measurement](VULKAN_DRIVER_MEASUREMENT_2026-11-27.md)).

A native driver process would run that 17% untranslated, on its own threads, and
could use either driver. It would not touch what limits the frame today, which is
the game's own main thread: 70% busy with game code that Rosetta translates.

## What it would be

```
host process (x86-64, Rosetta 2)            driver process (arm64)
┌ game: librendersystemvulkan ───────┐      ┌ decoder ────────────────────┐
│ Vulkan calls                       │      │ replays each call           │
├ encoder: serializes every call ────┤ ring │ MoltenVK or KosmicKrisp     │
│ into shared memory                 │ ───▶ │ Metal                       │
│ replies only where a call returns  │ ◀─── │ the window's drawable       │
│ data                               │      │                             │
└────────────────────────────────────┘      └─────────────────────────────┘
```

1. **An encoder in the guest's domain.** Every Vulkan function the game gets
   becomes one that writes its arguments, and what they point at, into a ring of
   shared memory. It is neutral code: no GS switch at all.
2. **A transport.** The ring, a way to wake the other side, and a reply path for
   the calls that return data (creation calls, queries, `vkMapMemory`). Memory the
   game maps must be the pages the driver uses, so buffers need memory both
   processes map.
3. **A decoder in the driver process**, which makes the real calls, and owns
   presentation.

## What exists to build it from

- **Venus**, Mesa's Vulkan driver for virtual machines, is exactly an encoder: it
  serializes Vulkan calls for a renderer on the other side, virglrenderer, which
  replays them on the host's driver. (REPORTED, Mesa documentation)
- **It runs without a virtual machine.** virglrenderer has a test server,
  `virgl_test_server --venus`, that a Venus driver started with `VN_DEBUG=vtest`
  reaches over a socket; Mesa's documentation gives it as the simplest way to try
  Venus. (REPORTED, Mesa documentation)
- **Its renderer runs on macOS over MoltenVK.** krunkit, built on libkrun, gives
  Linux virtual machines on Apple silicon a GPU through Venus (REPORTED, Lima
  documentation). QEMU patches of 2026 let Venus run on macOS with MoltenVK and no
  OpenGL, and UTM's author posted a series for the same in December 2025, with the
  bulk of the work going to virglrenderer. (REPORTED, the patch series)

So the decoder half exists, natively, for the very driver cs2metal uses, though as
work still landing upstream rather than a release. The encoder half exists as a
Linux x86-64 library, which is what cs2metal loads as guest code.

## What stands in the way

| Obstacle | Why it is one |
| --- | --- |
| The window system integration | Venus implements swapchains in the guest, on X11 or Wayland, with images handed over as dma-bufs. The game has neither: it asks SDL for a surface, and host SDL makes a Metal one. A swapchain of the driver process would have to be invented: either that process owns the window, and then input arrives in the wrong process, or it renders into IOSurfaces that the host process shows, which adds a compositing step to every frame |
| The vtest transport | It passes file descriptors over a Unix socket and maps shared memory made with `memfd_create`. The system call layer has neither descriptor passing nor `memfd_create` |
| Page size | The host process has 4 KiB pages, a native one 16 KiB. Memory both map must be laid out for both; the QEMU series names a page-granule constraint of its own (REPORTED) |
| The Steam overlay | It is inserted into the process that presents. In a driver process it would have to be inserted there, by a start the Steam client does not make |
| Maturity | Venus on macOS is patches under review, not a supported configuration. A 2023 report had the guest crash at `vkCreateInstance` over MoltenVK; whether that is fixed was not checked (REPORTED) |
| One more process to keep alive | A driver process that dies takes every Vulkan object with it; the game cannot recover from that |

An own encoder and decoder, generated from the Vulkan registry for the 774
commands the renderer names, avoids Venus's assumptions about Linux but is the
whole of Venus's work done again.

## What it could gain

- The 17% of driver work would run native. How much faster is **UNKNOWN**: no
  measurement here compares MoltenVK native with MoltenVK translated under load.
  The one probe there is, a trivial scene, shows equal frame times
  ([driver measurement](VULKAN_DRIVER_MEASUREMENT_2026-11-27.md)).
- That work would leave the game's render threads, which would only encode. If
  those threads limit the frame, this is a real gain; the samples say the main
  thread does, not they. (INFERRED)
- KosmicKrisp would become usable, with `VK_EXT_mutable_descriptor_type`,
  `VK_EXT_multi_draw`, and `VK_EXT_conditional_rendering`. Neither driver offers
  graphics pipeline libraries, so the first-use stutters would stay.
- The GS switches of Vulkan calls would be gone. Recording commands on the guest's
  side ([Vulkan bridge](VULKAN_BRIDGE.md)) already removes most of them, for a
  fraction of the work.

## What would have to be measured first

1. **Where the frame is limited after the cheap work is in.** With the vDSO, the
   system call fast path, and command recording in place: is any render thread
   busy more than the main thread? If not, stop here.
2. **Native against translated MoltenVK under a real load.** The game's own
   command stream cannot be replayed natively without this route, so a stand-in: a
   Vulkan benchmark built for both architectures, with thousands of draws per
   frame.
3. **A spike of the transport**: virglrenderer's vtest server built for arm64
   macOS, and a Linux fixture that draws a triangle through Mesa's Venus driver
   under cs2metal, offscreen. It shows the system calls the transport needs and
   whether the page sizes meet.
4. **A swapchain through IOSurface**, timed: what the extra compositing step costs
   per frame at 2560 by 1440.

Each is days, not weeks, and each can end the study.

## Assessment

Weeks of work at the least, on a base that is not released, to speed up a part of
the frame that does not limit it today. It is the right route only if step 1
shows the render threads as the limit, and it is the only route to a native
driver while the host is a Rosetta process. It stays a study. The entry in
[future work](FUTURE_WORK.md) points here.

## Sources

- [Mesa documentation, Virtio-GPU Venus](https://docs.mesa3d.org/drivers/venus.html)
- [Lima documentation, krunkit](https://lima-vm.io/docs/config/vmtype/krunkit/)
- [QEMU patch series, enable Venus without an OpenGL display, March 2026](https://patchew.org/QEMU/20260309214930.92616-1-lucaaamaral@gmail.com)
- [QEMU patch series, Venus support for macOS, December 2025](https://patchew.org/QEMU/20251203040754.94487-1-j@getutm.app)
- [mesa-dev, Venus crashes guest if vkCreateInstance is called on MoltenVK, June 2023](https://lists.freedesktop.org/archives/mesa-dev/2023-June/226015.html)
