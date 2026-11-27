# cs2metal

An experimental compatibility host that runs the **native Linux build** of
Counter-Strike 2 on Apple silicon Macs, without Wine and without a virtual machine.

> [!WARNING]
> **A research prototype that runs.** As of 27 November 2026 the game boots to its
> main menu and plays a practice match with bots on an Apple M1 Max, with keyboard,
> mouse, and sound, at 50 to 80 frames per second depending on the settings, always
> offline with `-insecure`
> ([what works and what does not](docs/CLIENT_PLAYABLE_2026-11-27.md)). Expect
> stutters, rough edges, and breakage with any game update.
> [docs/GETTING_STARTED.md](docs/GETTING_STARTED.md) puts the pieces on a Mac, and
> [docs/ROADMAP.md](docs/ROADMAP.md) is the path that led here.
> The project is not affiliated with or endorsed by Valve or Apple.

## The idea in one paragraph

Valve ships CS2 for Windows and Linux only. The Linux build is a set of x86-64 ELF
shared objects that render through Vulkan and talk to the system through glibc and
SDL. Linux x86-64 and macOS x86-64 share the System V calling convention and the
Itanium C++ ABI, so an x86-64 macOS process running under Rosetta 2 can map those
modules, bind their imports to our own implementation of the glibc ABI, hand their
SDL and Vulkan calls to macOS builds of the same libraries, and reach Metal through a
Vulkan-on-Metal driver. The game files are never modified on disk.

## Rules that do not bend

- Game files are read-only and byte-identical forever. The host verifies them against
  a baseline before every launch.
- No Steam credentials, session files, or game files are ever tracked.
- The project never studies, disables, or spoofs Valve Anti-Cheat.
- Every in-memory compatibility transform is named, documented, and counted at load.

## Building

You need an Apple silicon Mac and [rustup](https://rustup.rs).

```bash
tools/check.sh                        # format, lint, tests, guard and documentation checks
cargo run -p cs2metal -- --help
```

## Running the game

After the steps of [docs/GETTING_STARTED.md](docs/GETTING_STARTED.md), with the
rebuilt glibc in `target/guest-glibc` (`tools/guest-glibc.sh`), the integrity
baseline recorded, and Steam for macOS running and signed in:

```bash
cargo build --release --target x86_64-apple-darwin -p cs2metal
target/x86_64-apple-darwin/release/cs2metal run \
  --game-dir <game_dir> --runtime-dir <runtime_dir>/files \
  -- -insecure
```

`run` verifies every game file against the baseline first, and sends the game's
writes to an overlay directory beside the installation. Always pass `-insecure`:
the project never joins a VAC-secured server. Diagnostics are environment
variables, described where they belong: `CS2METAL_SDL_TRACE`
([SDL bridge](docs/SDL_BRIDGE.md)), `CS2METAL_VULKAN_TRACE`
([Vulkan bridge](docs/VULKAN_BRIDGE.md)), and `CS2METAL_MAPS`,
`CS2METAL_FUTEX_TRACE`, and `CS2METAL_STACKS`
([architecture](docs/ARCHITECTURE.md)).

## Inspecting a Linux build

```bash
cargo run -p cs2metal -- baseline record <game_dir>      # SHA-256 of every file, to baselines/
cargo run -p cs2metal -- baseline verify <game_dir>      # exit status 4 on any difference
cargo run -p cs2metal -- tls scan <game_dir>             # fs accesses and unwind coverage
cargo run -p cs2metal -- inspect <game_dir>/game/bin/linuxsteamrt64/libtier0.so
cargo run -p cs2metal -- inventory <game_dir> --markdown reports/inventory.md --json reports/inventory.json
```

`inventory` walks a CS2 Linux installation, parses every ELF object, resolves each
import against the bundled modules, and lists what the host must provide. Reports go
to `reports/`, which is ignored: they are derived from game files and stay local.

## Layout

| Path | Contents |
| --- | --- |
| `crates/cs2metal-elf` | Bounds-checked, read-only ELF64 parser that reads what a dynamic loader reads |
| `crates/cs2metal-inventory` | Installation scan, import resolution, and host requirement report |
| `crates/cs2metal-integrity` | Integrity baseline: record and verify the SHA-256 of every game file |
| `crates/cs2metal-codescan` | Code scan: `fs` accesses, unwind coverage, and TLS models per module |
| `crates/cs2metal-loader` | Maps guest ELF modules, rewrites `fs` to `gs`, and bridges the guest and host domains |
| `crates/cs2metal-linux` | The Linux system call layer on macOS, and the Steam, SDL, and Vulkan bridges |
| `crates/cs2metal-vulkan` | Static scan of the renderer's Vulkan requirements |
| `guest/` | The rebuilt glibc's patch and build, and the shim libraries served to the game |
| `apps/cs2metal` | Command-line front end |
| `spikes/` | Small, self-contained platform probes and their reproduction scripts |
| `tools/` | Repository guard and the check suite every commit must pass |
| `docs/` | Architecture, decision records, implementation plan, research, and policies |

## Documentation

Architecture, decision records, the implementation plan, and dated research live in
[`docs/`](docs/README.md). Start with the index there.

## Licence

cs2metal is free software, licensed under the
[GNU General Public License, version 3](LICENSE). It comes with no warranty. The
licence covers this project's own code and documentation only; Counter-Strike 2 and
Steam belong to Valve and are never distributed here.
