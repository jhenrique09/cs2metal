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

- An Apple silicon Mac on macOS 26 or newer, with about 80 GB free. The download is
  about 60 GB and takes 71 GB on disk.
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
  bin/     cs2metal, the host program
           guest/               the guest's C library and the shims
           run.sh, run-online.sh, create-shortcut.sh, and their helpers
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
~/cs2-metal/bin/run.sh
```

Steam for macOS must be running and signed in. The game starts offline, with
`-insecure`. To stop it, quit from its menu or press Ctrl-C in the terminal: both
let the game shut itself down.

| Before the game starts, `run.sh` | Turn it off with |
| --- | --- |
| Asks Steam whether there is a newer build and installs it (see [Update](#update)) | `CS2METAL_NO_UPDATE=1` |
| On the first launch only, sets the game's video settings to its own low preset | `CS2METAL_KEEP_VIDEO_SETTINGS=1` |
| Loads Steam's overlay (Shift+Tab) into the host | `CS2METAL_NO_OVERLAY=1` |
| Leaves the opening movie out | `CS2METAL_KEEP_INTRO=1` |

Arguments go to the engine: `run.sh +map de_dust2`. Diagnostic variables such as
`CS2METAL_SDL_TRACE=1` and `CS2METAL_VULKAN_TRACE=1` pass through to the host and
write to the session's log.

`run-online.sh` starts the game without `-insecure`, so that it can join servers
secured by Valve Anti-Cheat. How that anti-cheat treats this host is unknown. Read
the [decision record](ONLINE_LAUNCH_DECISION.md) first, and use a secondary account.

## A shortcut

```bash
~/cs2-metal/bin/create-shortcut.sh            # ~/Applications/Counter-Strike 2.app
~/cs2-metal/bin/create-shortcut.sh --steam    # and add it to Steam's library
~/cs2-metal/bin/create-shortcut.sh --online   # an application that runs run-online.sh
```

The application runs `run.sh`. Its icon is converted on your Mac from the game's own
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

1. **The host program**, with cargo, for `x86_64-apple-darwin`.
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
- **A long black screen at start.** The very first start, and the first after
  `build.sh` installed a new host program or after the shader caches were lost,
  compiles the game's shaders for Metal again and can take two minutes. Let it
  finish: later starts take seconds. Do not force-quit the game while it does.
- **Stutters in the first matches.** Each kind of scene is compiled the first time
  it is drawn. They fade as you play.
- **To report a problem**, attach the session's log from `~/cs2-metal/logs/`, taken
  with `CS2METAL_SDL_TRACE=1 CS2METAL_VULKAN_TRACE=1` if it concerns input, sound,
  or frame pacing.
