# cs2metal

An experimental compatibility host that runs the **native Linux build** of
Counter-Strike 2 on Apple silicon Macs, without Wine and without a virtual machine.

> [!WARNING]
> **Nothing runs yet.** The project is at milestone M0: repository foundations, a
> read-only ELF inventory tool, and the first platform probes. It does not start the
> game. It is not affiliated with or endorsed by Valve or Apple.

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

## Inspecting a Linux build

```bash
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
