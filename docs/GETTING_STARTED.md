# Getting started

## Status

Current as of 27 November 2026. This is how to set up, start, update, and remove
Counter-Strike 2 under cs2metal on an Apple silicon Mac, with the scripts at the top
of the repository.

What has been run, on the owner's Apple M1 Max, where the pieces were already
installed by hand before the scripts existed:

- `build.sh`, from start to end, including the build of the C library from source
  in Docker (11 minutes 35 seconds, the checksums matching), the path that takes
  the library from a directory, and the message when neither is possible.
- `run.sh`, from `~/cs2-metal/bin`, to the game's menu and a clean quit.
- `create-shortcut.sh`, without `--steam`.
- `install.sh`, with every download already done: it found each piece, built, and
  verified the baseline.

What has not: the download steps of `install.sh` on an empty Mac (they are the
commands of the manual setup, which worked), the update of the game by `run.sh`
(Steam offered no newer build that day), and `create-shortcut.sh --steam`. Expect
to be the first to run those, and report what happens.

## What you need

- An Apple silicon Mac with about 80 GB free: the download is about 60 GB and
  takes 71 GB on disk. Everything here was run on macOS 27 only.
- A Steam account. Use the one the [policy](ANTI_CHEAT_AND_INTEGRITY_POLICY.md)
  allows: a secondary account.

`install.sh` checks for the rest and says how to install what is missing:

| Needed | For | How |
| --- | --- | --- |
| Rosetta 2 | The host is an x86-64 process | `softwareupdate --install-rosetta --agree-to-license` |
| Xcode command line tools | The compiler driver, `objdump`, `python3` | `xcode-select --install` |
| rustup | Building the host | <https://rustup.rs> |
| Docker, running | One step of the build, once: the guest's C library, in a Linux container. Not needed if you have the six libraries already (see [Build](#build)) | [OrbStack](https://orbstack.dev) or Docker Desktop |
| Vulkan SDK for macOS | MoltenVK, the Vulkan loader, and SDL 3, as x86-64 libraries | The installer from <https://vulkan.lunarg.com/sdk/home>, in its default place, `~/VulkanSDK` |
| Steam for macOS, signed in | The Steam the game talks to | <https://store.steampowered.com/about/> |

**Do not install Counter-Strike 2 from Steam for macOS.** That client downloads the
game's shared content without the Linux programs, which are what cs2metal runs.

## Install

From a checkout of this repository:

```bash
./install.sh
```

It can be stopped and run again; it skips what is already done. In order, it:

1. Checks the Mac for the things above.
2. Waits until Steam for macOS is installed, running, and signed in.
3. Downloads steamcmd, Valve's command-line downloader.
4. Downloads the Linux build of the game with steamcmd, after asking. steamcmd
   signs in anonymously, which is enough for this game; set
   `CS2METAL_STEAM_LOGIN=<account>` to download as your own account, and steamcmd
   then asks for the password and the Steam Guard code itself. No project script
   sees a credential.
5. Downloads the Steam Linux Runtime 3.0 ("sniper"), 305 MB, checks it against
   Valve's checksum, and keeps the two C++ libraries the game needs.
6. Runs `./build.sh` (see [Build](#build)).
7. Records the integrity baseline: the SHA-256 of every game file. cs2metal verifies
   the installation against it before every start and never writes to it.
8. Asks whether to make an application shortcut, and whether to add it to Steam.

## Where things are

```
~/cs2-metal/
  bin/     cs2metal.app/        the host program, in an application bundle
           guest/               the guest's C library and the shims
           run.sh, run-offline.sh, create-shortcut.sh, and their helpers
  data/    steamcmd/            Valve's downloader
           cs2-linux/           the game, read-only to cs2metal
           cs2-linux.overlay/   everything the game writes into its installation
           sniper/              the C++ runtime
           baselines/           the integrity baseline
           first-launch         marks that the first launch has happened
  logs/    one log per session
```

The game's settings and saved configuration are where Steam keeps them for the
account, under Steam's own `userdata`. Nothing is installed anywhere else, except
the application shortcut if you ask for one.

## Play

```bash
~/cs2-metal/bin/run.sh             # online
~/cs2-metal/bin/run.sh -offline    # offline: bots and unsecured servers only
```

Steam for macOS must be signed in; if it is not running, `run.sh` opens it and
waits half a minute before it starts the game. If Steam asks for a password
instead of signing in by itself, sign in and start the game again.

**The first start stays black for a few minutes** while Metal compiles the game's
shaders, once; it is not stuck. To stop the game, quit from its menu or press
Ctrl-C in the terminal: both let it shut itself down.

### Online is the default, and it is at your own risk

`run.sh` starts the game as Steam would, so it can join matchmaking and servers
secured by Valve Anti-Cheat, on the account signed in to Steam
([decision](ONLINE_DEFAULT_DECISION.md)).

It works: on 27 November 2026 the owner played casual and competitive matches
online, against other players, and the account was not banned
([record](ONLINE_PLAY_2026-11-27.md)). That is one machine and one day. Valve does
not support this host, how Valve Anti-Cheat treats it is unknown, and the project
never tries to find out. A ban, if one came, could come later, and would be the
account's. Read the [policy](ANTI_CHEAT_AND_INTEGRITY_POLICY.md) first, and play
online only with an account you can afford to lose.

`run.sh -offline`, or `run-offline.sh`, passes `-insecure` to the game, which keeps
it off secured servers: practice matches with bots, with no such risk.

### What `run.sh` does first

| Before the game starts, `run.sh` | Change it with |
| --- | --- |
| Opens Steam for macOS if it is not running, and waits 30 seconds for it to sign in | `CS2METAL_STEAM_WAIT=<seconds>` |
| Asks Steam whether there is a newer build and installs it (see [Update](#update)) | `CS2METAL_NO_UPDATE=1` |
| On the first launch only, sets the game's video settings to its own low preset, with CMAA2 anti-aliasing on | `CS2METAL_KEEP_VIDEO_SETTINGS=1` |
| Loads Steam's overlay (Shift+Tab) into the host | `CS2METAL_NO_OVERLAY=1` |
| Leaves the opening movie out | `CS2METAL_KEEP_INTRO=1` |

Other arguments go to the engine: `run.sh +map de_dust2`. Diagnostic variables such
as `CS2METAL_SDL_TRACE=1` and `CS2METAL_VULKAN_TRACE=1` pass through to the host
and write to the session's log.

### Keep the shader caches: quit the game, never kill it

Two caches hold what the first sessions compiled, and later sessions are smooth
because of them: the game's own pipeline cache, among what it writes
(`~/cs2-metal/data/cs2-linux.overlay/game/csgo/shadercache/`), and Metal's compiled
shaders, in the user's cache directory (`com.apple.metal`). Three habits keep them:

- **Quit, do not kill.** Quit from the game's menu, or with Ctrl-C in the terminal
  that started it; both let the game shut itself down. Do not use `kill -9` or
  Force Quit. After test boots that were ended in the middle of their work, the
  next start spent two minutes on a black screen compiling its Metal shaders
  again (seen on 27 November 2026; the cause is INFERRED from the order of events).
- **Leave the host program where it is.** Do not move or copy
  `~/cs2-metal/bin/cs2metal.app` and start it from another path. Its first boot from a
  path it never ran from sat four minutes on a black screen while Metal compiled
  every shader again (seen on 27 November 2026). Building it again at the same
  path did not do that.
- **Warm a map up offline before playing it online.** The first minutes on a map
  never played stutter while its pipelines are compiled for the first time: 16 of
  the 28 stalls of one online match fell in its first minute
  ([measurements](PERFORMANCE_AB_2026-11-27.md)). A few minutes of
  `run.sh -offline +map de_mirage` with bots leave both caches filled for that map.

## A shortcut

```bash
~/cs2-metal/bin/create-shortcut.sh            # ~/Applications/Counter-Strike 2.app
~/cs2-metal/bin/create-shortcut.sh --steam    # and add it to Steam's library
~/cs2-metal/bin/create-shortcut.sh --offline  # a second application, which starts offline
```

The application runs `run.sh`: it opens Steam if Steam is not running, updates the
game if there is an update, and starts the game online. It shows in Spotlight,
Launchpad, and the Dock under the game's name. Its icon is converted on your Mac from the game's own
icon file; nothing of Valve's is shipped with cs2metal. `--steam` adds it to the
non-Steam games of every Steam account on the Mac. Steam must be quit for that,
because it writes its list of shortcuts when it quits; the list is copied first
([decision](STEAM_SHORTCUT_DECISION.md)).

## Update

- **The game.** `run.sh` compares the installed build with the one Steam offers. If
  they differ, steamcmd updates the installation and checks every file, the
  integrity baseline is recorded again for the new build, and the SDL shim is
  checked against the new game's SDL. If the game's SDL changed, `run.sh` stops and
  asks for `./build.sh`, which builds the shims again. An update of the game can
  also break cs2metal itself; nothing checks for that.
- **cs2metal.** Pull the repository and run `./build.sh`.

## Build

```bash
./build.sh
```

It builds three things on this Mac and installs them to `~/cs2-metal/bin`:

1. **The host program**, with cargo, for `x86_64-apple-darwin`, installed inside
   an application bundle, `~/cs2-metal/bin/cs2metal.app`. The bundle's `Info.plist`
   is what tells macOS that the program is a game: it names a games category and
   asks for Game Mode, which gives a full-screen game the processor and the GPU
   ahead of background work. macOS offers Game Mode to an application that
   declares itself this way and decides by itself whether to turn it on; a
   program outside a bundle is never offered it. The keys are not inherited by a
   program another one starts, which is why the host itself is the bundle's
   program. The game's icon is converted on your Mac from the game's own file.
2. **The guest's C library**: Debian's glibc 2.31 with the project's patch
   (`guest/glibc/README.md`), which sends every system call through one slot the
   host answers. Its six libraries have checksums in the repository, and nothing
   is installed that does not match them. `build.sh` takes them from the first
   place that has them:
   - `target/guest-glibc/` in the checkout, where an earlier build left them;
   - the directory named by `CS2METAL_GLIBC_FROM`, if you set it: six libraries
     built on another machine;
   - a build from source, in a pinned Steam Runtime SDK image with Docker. glibc
     builds only with GCC and GNU binutils for Linux, so this cannot run on macOS
     itself. Under emulation on an M1 Max it takes about twelve minutes, once.
3. **The shims**: small Linux libraries served in place of the game's SDL, the
   Vulkan loader, and the Steam client library. Apple's clang and the linker that
   ships with Rust build them in a few seconds, with no container. They are
   generated from the functions the game's SDL and this Mac's Steam client export,
   so the game and Steam must be installed first.

So Docker is needed for one step, the first time, and not at all with
`CS2METAL_GLIBC_FROM`:

```bash
CS2METAL_GLIBC_FROM=<directory> ./build.sh
```

`--force-guest` builds the C library from source again, and `--host-only` builds
and installs only the host program. `tools/check.sh` runs the checks every commit
must pass.

## Remove

```bash
rm -rf ~/cs2-metal "$HOME/Applications/Counter-Strike 2.app"
docker volume rm cs2metal-glibc-work     # only if the C library was built here
```

That removes the game, the runtime, the logs, and the build's working copy of the
glibc source. Two things outside it are left: the game's own settings in Steam's
`userdata/<account>/730`, and, if you added the shortcut to Steam, its entry there
(remove it from Steam's library).

## If something goes wrong

- **The game does not start, and the log ends with a count of differences.** The
  installation no longer matches its baseline. If you did not change it, let
  steamcmd repair it and record the baseline again:
  `~/cs2-metal/bin/run.sh` does both when Steam offers a different build; otherwise
  run `./install.sh`, which records a baseline when the old one does not match.
- **A long black screen at the first start.** The very first start compiles the
  game's shaders for Metal before the menu appears: about four minutes on an M1
  Max on 27 November 2026, on a black screen, with macOS's shader compiler busy
  and the game's log standing still. Let it finish: later starts take seconds.
  The same happens again if the shader caches are lost, which a force-quit game
  has caused here, so quit from the menu or with Ctrl-C.
- **Stutters in the first matches.** Each kind of scene is compiled the first time
  it is drawn. They fade as you play.
- **To report a problem**, attach the session's log from `~/cs2-metal/logs/`, taken
  with `CS2METAL_SDL_TRACE=1 CS2METAL_VULKAN_TRACE=1` if it concerns input, sound,
  or frame pacing.
