# cs2metal

Counter-Strike 2 on Apple silicon Macs, by running the game's **native Linux build**
inside one macOS process. No Wine, no virtual machine, no modified game file.

> [!WARNING]
> **It works, and it is at your own risk.** As of 27 November 2026, on an Apple M1
> Max, the game runs from its main menu through practice matches with bots and, as
> its owner reports, casual and competitive matches online, with no VAC ban so far.
> That is one machine and one day. Valve does not support this. How Valve Anti-Cheat
> treats this host is **unknown**, and a ban, if one came, need not come the same
> day. **The game starts online unless you start it with `-offline`**, so play only
> with an account you can afford to lose. The project is not affiliated with or
> endorsed by Valve or Apple.

## What works

| | State on 27 November 2026 |
| --- | --- |
| Menu, practice matches with bots | Works ([record](docs/CLIENT_PLAYABLE_2026-11-27.md)) |
| Online: casual and competitive | Works, by the owner's report; no VAC ban so far ([record](docs/ONLINE_PLAY_2026-11-27.md)) |
| Graphics | The game's own Vulkan renderer, on Metal through MoltenVK. 77 to 127 frames per second on the low preset and 50 to 62 on the high one, at 2560 by 1440 on an M1 Max |
| Keyboard and mouse | Work, with the pointer captured in a match and text in the game's console |
| Sound | Works |
| Steam | The signed-in account of Steam for macOS: friends, inventory, the overlay (Shift+Tab), and the pages the game opens in Steam's browser |
| Game updates | `run.sh` installs them before starting |

What does not, yet:

- **The first start takes a few minutes on a black screen**, while Metal compiles
  the game's shaders, once. Later starts take seconds.
- **Stutters the first time each kind of scene is drawn.** Linux players get their
  shaders compiled ahead of time by Steam; here each one is compiled when first
  used. They fade as you play and return, less, in each new session.
- **The opening movie is left out.** Under this host it stood still on single
  frames, so a black screen takes its three seconds. `CS2METAL_KEEP_INTRO=1` plays it.
- **Game controllers and the microphone are not tried.**
- **It depends on Rosetta 2**, which Apple has said it will reduce after macOS 27
  ([decision](docs/ROSETTA_HORIZON_DECISION.md)).
- **A game update can break it** until the host catches up.

## Install

You need an Apple silicon Mac with about 80 GB free, and a Steam account. From a
checkout of this repository:

```bash
./install.sh
```

It checks what must be on the Mac (Rosetta 2, the Xcode command line tools,
[rustup](https://rustup.rs), the Vulkan SDK, and Docker for one step of the build)
and says how to get what is missing; has you install Steam for macOS and sign in;
downloads the Linux build of the game with Valve's steamcmd, and the C++ runtime it
needs; builds everything; and offers to make an application shortcut and add it to
Steam.

**Do not install Counter-Strike 2 from Steam for macOS.** That download has none of
the Linux programs this host runs. Steam itself is needed: it is the Steam the game
talks to.

[docs/GETTING_STARTED.md](docs/GETTING_STARTED.md) walks through every step, the
updates, the shortcut, and how to remove it all.

## Play

```bash
~/cs2-metal/bin/run.sh                 # online, at your own risk
~/cs2-metal/bin/run.sh -offline        # offline: bots and unsecured servers only
~/cs2-metal/bin/run.sh +map de_dust2   # extra arguments go to the engine
```

Quit from the game's menu, or press Ctrl-C in the terminal. Before the game
starts, `run.sh`:

- opens Steam for macOS if it is not running, and gives it half a minute to sign
  in with the account it remembers;
- asks Steam whether there is a newer build of the game, and installs it;
- on the first launch only, sets the game's video settings to its own low preset;
- loads Steam's own overlay into the host;
- writes a log for the session to `~/cs2-metal/logs/`.

The top of [`run.sh`](run.sh) lists the environment variables that change each of
these.

**`run.sh` starts the game online**, as Steam would: it can join matchmaking and
servers secured by Valve Anti-Cheat, on the account signed in to Steam. That is
the risk the warning at the top of this page describes; read
[the decision](docs/ONLINE_DEFAULT_DECISION.md) and
[the policy](docs/ANTI_CHEAT_AND_INTEGRITY_POLICY.md) first. `run.sh -offline`, or
`run-offline.sh`, starts it with `-insecure` instead, which keeps it off those
servers: practice matches with bots carry no such risk.

`~/cs2-metal/bin/create-shortcut.sh` makes `~/Applications/Counter-Strike 2.app`,
with the game's own icon: an application for Spotlight, Launchpad, and the Dock
that does what `run.sh` does, opening Steam first if it has to. With `--steam` it
is also added to Steam's library.

## Where everything is

| Path | Contents |
| --- | --- |
| `~/cs2-metal/bin/` | The host program `cs2metal`, the guest's C library and shims (`guest/`), and the scripts that start the game |
| `~/cs2-metal/data/` | steamcmd, the game (`cs2-linux/`), what the game writes (`cs2-linux.overlay/`), the C++ runtime (`sniper/`), and the integrity baseline |
| `~/cs2-metal/logs/` | One log per session |

Nothing else is installed, except the application shortcut if you ask for one.

## How it works

Valve ships CS2 for Windows and Linux only. The Linux build is a set of x86-64 ELF
programs and libraries that draw through Vulkan and talk to the system through glibc
and SDL. Linux and macOS share, on x86-64, the same calling convention and the same
C++ ABI. So one x86-64 macOS process, translated by Rosetta 2, can:

1. **Load the game as it is.** The host maps the game's ELF modules with the game's
   own dynamic loader. Not a byte of a game file changes on disk, and the host
   verifies every file against a recorded checksum before each start.
2. **Give it the C library it was built for.** The guest runs on glibc 2.31, as on
   Linux, rebuilt from Debian's source with one change: each system call goes
   through a slot that the host answers with macOS services, instead of reaching the
   macOS kernel as a Linux system call.
3. **Answer three libraries with macOS ones.** The game's SDL, the Vulkan loader,
   and the Steam client library are replaced at the boundary by small shims that
   forward every call to SDL 3 for macOS, to MoltenVK, and to the client library of
   the Steam for macOS you are signed in to.

```
 CS2 Linux build (unmodified ELF x86-64)      cs2 · libengine2 · libclient · ...
 guest C library                              glibc 2.31, one system call slot
────────────────────────────────────────────────────────────────────────────────
 cs2metal host (x86-64 process, Rosetta 2)    loader · system calls · bridges
────────────────────────────────────────────────────────────────────────────────
 macOS                                        SDL 3 · MoltenVK on Metal · Steam
```

One thing is changed in memory, and it is registered and counted at every load:
Rosetta cannot give a process the `fs` segment base Linux thread-local storage
uses, so `fs` prefixes in the game's code are rewritten to `gs` as each module is
mapped ([decision](docs/TLS_UNDER_ROSETTA_DECISION.md)).
[docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) has the whole design.

## Rules that do not bend

- Game files are read-only and byte-identical forever. The host verifies them against
  a baseline before every launch.
- No Steam credentials, session files, or game files are ever tracked. You sign in
  to Valve's own programs; no project script sees a password.
- The project never studies, disables, intercepts, or spoofs Valve Anti-Cheat.
- No gameplay changes and no advantage of any kind.
- Every in-memory compatibility transform is named, documented, and counted at load.

[The policy](docs/ANTI_CHEAT_AND_INTEGRITY_POLICY.md) states them in full.

## Build and develop

```bash
./build.sh               # build what changed and install it to ~/cs2-metal/bin
tools/check.sh           # format, lint, tests, guard and documentation checks
```

`build.sh` builds three things on this Mac:

| What | How | Needs |
| --- | --- | --- |
| The host program | cargo, for `x86_64-apple-darwin` | rustup |
| The shims | Apple's clang and the linker that ships with Rust; generated from the functions the game's SDL and your Steam client export | The Xcode command line tools |
| The guest's C library | Debian's glibc 2.31 with [the project's patch](guest/glibc/README.md), built from source in a pinned Linux container | Docker, once |

Only the last needs Docker, because glibc builds only with GCC and GNU binutils for
Linux. The build is reproducible: its six libraries come out byte-identical to the
checksums in the repository, on a Linux host and under emulation on a Mac, or the
build fails. It takes about twelve minutes on an M1 Max and happens once; after
that `build.sh` finds the libraries and checks them against the same checksums.

To build without Docker, give `build.sh` six libraries built on another machine:
`CS2METAL_GLIBC_FROM=<directory> ./build.sh` checks them against the same checksums
and installs nothing that does not match.

The host program can also be run by hand:

```bash
~/cs2-metal/bin/cs2metal run \
  --game-dir <game_dir> --runtime-dir <runtime_dir>/files \
  --glibc <guest_dir> --baseline <baseline.json> \
  -- -insecure
```

Diagnostics are environment variables, described where they belong:
`CS2METAL_SDL_TRACE` ([SDL bridge](docs/SDL_BRIDGE.md)), `CS2METAL_VULKAN_TRACE`
([Vulkan bridge](docs/VULKAN_BRIDGE.md)), and `CS2METAL_MAPS`,
`CS2METAL_FUTEX_TRACE`, and `CS2METAL_STACKS`
([architecture](docs/ARCHITECTURE.md)). With them a session's log shows the events
the game takes, the pacing of its frames, and where a thread that stopped is.

The same program inspects a Linux build without running it:

```bash
cs2metal baseline record <game_dir>      # SHA-256 of every file
cs2metal baseline verify <game_dir>      # exit status 4 on any difference
cs2metal tls scan <game_dir>             # fs accesses and unwind coverage
cs2metal inspect <game_dir>/game/bin/linuxsteamrt64/libtier0.so
cs2metal inventory <game_dir> --markdown reports/inventory.md --json reports/inventory.json
```

`inventory` walks an installation, parses every ELF object, resolves each import
against the bundled modules, and lists what a host must provide. Reports go to
`reports/`, which is ignored: they are derived from game files and stay local.

## Repository

| Path | Contents |
| --- | --- |
| `install.sh`, `build.sh`, `run.sh`, `run-offline.sh`, `create-shortcut.sh` | Set up, build, start, and make a shortcut for the game |
| `apps/cs2metal` | The command-line program: `run`, `baseline`, `inspect`, `inventory`, `tls`, `vulkan` |
| `crates/cs2metal-loader` | Maps guest ELF modules, rewrites `fs` to `gs`, and bridges the guest and host domains |
| `crates/cs2metal-linux` | The Linux system call layer on macOS, and the Steam, SDL, and Vulkan bridges |
| `crates/cs2metal-elf` | Bounds-checked, read-only ELF64 parser that reads what a dynamic loader reads |
| `crates/cs2metal-integrity` | Integrity baseline: record and verify the SHA-256 of every game file |
| `crates/cs2metal-inventory` | Installation scan, import resolution, and host requirement report |
| `crates/cs2metal-codescan` | Code scan: `fs` accesses, unwind coverage, and TLS models per module |
| `crates/cs2metal-vulkan` | Static scan of the renderer's Vulkan requirements |
| `guest/` | The guest C library's patch and build, and the shim libraries served to the game |
| `tools/` | The check suite every commit must pass, the repository guard, and the helpers the scripts above use |
| `fixtures/`, `spikes/` | Linux test programs the loader is tested with, and small platform probes |
| `docs/` | Architecture, decision records, the roadmap and plan, dated measurements, and policies |

## Documentation

[`docs/README.md`](docs/README.md) is the index. The shortest path through it:

- [Getting started](docs/GETTING_STARTED.md): install, play, update, remove.
- [Architecture](docs/ARCHITECTURE.md): how the host is built.
- [The client, playable offline](docs/CLIENT_PLAYABLE_2026-11-27.md) and
  [online play](docs/ONLINE_PLAY_2026-11-27.md): what was measured and reported.
- [Anti-cheat, integrity, and legal policy](docs/ANTI_CHEAT_AND_INTEGRITY_POLICY.md):
  the rules, and the risk.

## Licence

cs2metal is free software, licensed under the
[GNU General Public License, version 3](LICENSE). It comes with no warranty. The
licence covers this project's own code and documentation only; Counter-Strike 2 and
Steam belong to Valve and are never distributed here. The guest's C library is glibc,
under the LGPL; the project distributes its patch, not the library.
