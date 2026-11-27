# Feasibility research, 26 November 2026

## Status

Research snapshot of 26 November 2026. Feeds the
[Linux build decision](LINUX_BUILD_BASELINE_DECISION.md), the
[architecture](ARCHITECTURE.md), and the [implementation plan](IMPLEMENTATION_PLAN.md).
Not edited after the fact.

Evidence tags follow the documentation policy in [README.md](README.md).

## 1. Prior art and what each one teaches

| Project | Route | Lesson for cs2metal |
| --- | --- | --- |
| [VIctory for macOS](https://github.com/pragyaangaur/VIctory-macOS) | Xbox Series GTA VI build under CrossOver Wine and D3DMetal | A compatibility stack built on layers the project does not own hits limits it cannot fix: memory visibility between Wine and D3DMetal, Wine's single memory lock, deadlocks inside Wine. (REPORTED in its README.) |
| [Native VIctory for macOS](https://github.com/LakshanSivakumar/Native-Victory-macOS) | Own loader and about 1,000 reimplemented Windows and Xbox functions in Rust, GPU straight to Metal, game code under Rosetta | The model this project follows. Under Rosetta, `thread_fast_set_cthread_self` moves the GS base, about 190 ns per call on their machine, and the host TSD base sits at `pthread_self() + 0xE0`; masked AVX stores that fault cannot be resumed. (REPORTED in its `docs/native-host.md`; the GS mechanism is CONFIRMED here by the [probe](ROSETTA_SEGMENT_PROBE_2026-11-26.md).) |
| [cs2-apple-silicon](https://github.com/piwas-21/cs2-apple-silicon) | CS2 Windows build in a Wine bottle with DXMT (Direct3D 11 to Metal) | The Windows route already works: about 128 fps on the Ancient benchmark at low settings on an M2 Pro, 48 minutes of bot matches without crashes. Competitive play is not validated. Its research records Valve's depot layout and the steamcmd platform override. (REPORTED in its README and research notes.) |

What sets cs2metal apart: none of the three runs the **Linux** build, and only the
Native VIctory route avoids an emulated operating system entirely.

## 2. The CS2 Linux build

- **Platforms.** Steam's app metadata for app 730 lists `oslist = windows,linux`; the
  storefront reports `mac: false`. (REPORTED by cs2-apple-silicon from Valve's
  appinfo and store APIs.)
- **Depots.** Depot 2347773 is the Linux-only depot: about 4.6 GB to download and
  7.2 GB on disk, holding the launcher script and the ELF binaries. The content
  depots (2347770, 2347774, 2347777) are shared by every platform. (REPORTED, same
  source.)
- **Runtime.** The Linux build runs inside Steam Linux Runtime 3.0 ("sniper").
  (REPORTED by secondary sources.) The runtime matters because it, not the host
  distribution, supplies glibc and the C++ runtime the game was built against.
- **Libraries.** A user log on the Arch Linux forums shows the game loading
  `libSDL3.so.0` and `librendersystemvulkan.so` from `game/bin/linuxsteamrt64`.
  (REPORTED, single source.) Whether SDL3 and the C++ runtime are bundled or come
  from the runtime is **UNKNOWN** until the inventory runs on a real installation.
- **Graphics.** The Linux client renders through Vulkan, and Valve recommended
  `VK_EXT_graphics_pipeline_library` at launch. (REPORTED by
  [Phoronix](https://www.phoronix.com/news/Counter-Strike-2-Linux) and others.) The
  full list of required Vulkan features and extensions is **UNKNOWN**.
- **Main executable type.** Whether `cs2` is a position-dependent `ET_EXEC` linked at
  a low fixed address or a PIE is **UNKNOWN**. It matters: macOS x86-64 processes
  reserve the first 4 GB as `__PAGEZERO` by default, so a fixed low address needs a
  host linked with a small page-zero, as Wine's macOS loader does. (INFERRED.)

## 3. Getting the Linux build on a Mac

- macOS Steam never downloads depot 2347773 and cannot launch app 730. (REPORTED by
  cs2-apple-silicon, which measured the download queue byte for byte.)
- `steamcmd` accepts `+@sSteamCmdForcePlatformType linux`, documented on Valve's
  developer wiki, and app 730 requires a logged-in account. (REPORTED, same source.)
  The owner runs steamcmd and signs in; project tools never see credentials.

## 4. Platform facts

- **Rosetta's horizon.** Apple states that macOS 27 is the last release in which
  Rosetta is a general-purpose tool and that a subset stays available afterwards,
  including for some older, unmaintained games
  ([Apple developer news](https://developer.apple.com/news/?id=w5ngl9k2)). Which
  processes the subset will cover is **UNKNOWN**. Upgrades to macOS 27.2 no longer
  remove an installed Rosetta
  ([MacRumors, 27 October 2026](https://www.macrumors.com/2026/10/27/macos-27-2-stops-removing-rosetta/)).
  The development machine runs macOS 27.2.1. (CONFIRMED.)
- **Vulkan on Metal.** LunarG's KosmicKrisp, a Mesa driver, is conformant to Vulkan
  1.4 on Apple silicon
  ([Khronos](https://www.khronos.org/news/permalink/kosmickrisp-achieves-vulkan-1.4-conformance-on-apple-silicon)).
  It needs an M1 or newer and macOS 26 or newer, because it targets Metal 4
  ([Mesa documentation](https://docs.mesa3d.org/drivers/kosmickrisp.html)).
  (REPORTED.) MoltenVK remains the alternative. Whether an x86-64 build of either can
  be loaded into a Rosetta process with acceptable performance is **UNKNOWN**.
- **Calling convention.** Linux x86-64 and macOS x86-64 both use the System V AMD64
  calling convention, and both C++ toolchains follow the Itanium C++ ABI. Guest code
  can therefore call host C functions directly once data layouts agree, unlike the
  Windows route, where every crossing converts between the Win64 and System V
  conventions. (CONFIRMED by the published ABIs.)
- **Where layouts differ.** glibc and macOS disagree on many types the game passes
  across the boundary: `struct stat`, `struct dirent`, `sigset_t` (128 bytes against
  4), `pthread_mutex_t` (40 bytes against 64), `pthread_cond_t`, errno values, open
  and mmap flags, signal numbers, and `FILE`. (CONFIRMED by the headers.) libstdc++
  and libc++ are not layout-compatible (`std::string` alone differs), so the guest
  must keep a Linux C++ runtime. (CONFIRMED by their published layouts.)

## 5. Open questions

| Id | Question | How it closes |
| --- | --- | --- |
| Q-1 | Is `cs2` a fixed-address `ET_EXEC` or a PIE? | Inventory on a real installation (M1). |
| Q-2 | Which libraries does the build bundle, and which does it expect from the runtime? | Inventory (M1). |
| Q-3 | How many distinct glibc symbols, and which symbol versions, does the build import? | Inventory (M1). |
| Q-4 | Do bundled modules interpose allocator symbols such as `malloc`? | Inventory (M1). |
| Q-5 | Which TLS models do the modules use (static TLS, TLSDESC, dynamic)? | Inventory relocation counts (M1). |
| Q-6 | Can an x86-64 Vulkan driver (KosmicKrisp or MoltenVK) run in a Rosetta process and present to a window? | A probe in `spikes/` (M6). |
| Q-7 | Which Vulkan features and extensions does `librendersystemvulkan.so` require? | Its own capability queries, logged by the host (M6). |
| Q-8 | Does the macOS Steamworks library serve a Linux guest through the same flat API and interface tables? | A probe against the owner's Steam client (M7). |
| Q-9 | What replaces Rosetta after macOS 27 for this process? | Research gate M9, before fall 2027. |
