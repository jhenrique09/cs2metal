# Getting started

## Status

Current as of 27 November 2026. This is the owner's checklist for the first session:
seven steps that put every piece on the Mac and end with the first inventory of the
Linux build. It covers phases A and B of the [roadmap](ROADMAP.md), which explains
the reasons behind each step. Nothing here starts the game. The host that runs it
is built: once these steps are done, the repository's `README.md` gives the command
that starts the game.

## Before you begin

You need an Apple silicon Mac on macOS 26 or newer with Rosetta 2, the Xcode command
line tools, [rustup](https://rustup.rs), and about 80 GB free. Only step 4 takes
long: it downloads about 60 GB.

Everything downloaded goes into one directory outside this repository, so that Git
never sees a game file:

```
~/cs2metal-kit/
  steamcmd/     Valve's command-line downloader, with its own sign-in cache
  cs2-linux/    the Linux build of CS2; cs2metal only ever reads it
  sniper/       the C++ runtime from Steam Linux Runtime 3.0
```

Steps 2 and 4 need your Steam credentials. You type them into Valve's own programs;
no project tool ever sees them. Use the account the
[policy](ANTI_CHEAT_AND_INTEGRITY_POLICY.md) allows: a secondary one.

## 1. Add the x86-64 Rust target

The cs2metal host is an x86-64 process that runs under Rosetta.

```bash
rustup target add x86_64-apple-darwin
```

## 2. Install Steam for macOS

```bash
curl -fL -o ~/Downloads/steam.dmg https://cdn.fastly.steamstatic.com/client/installer/steam.dmg
open ~/Downloads/steam.dmg
```

Drag Steam to Applications, start it, and sign in.

**Do not install CS2 from this client.** Steam for macOS downloads the shared content
without the Linux binaries. The client is needed later, as the Steam the game talks
to at run time.

## 3. Download steamcmd

Valve's command-line downloader, as a tarball from Valve's CDN. Nothing is installed
system-wide.

```bash
mkdir -p ~/cs2metal-kit/steamcmd && cd ~/cs2metal-kit/steamcmd
curl -sSfL https://steamcdn-a.akamaihd.net/client/installer/steamcmd_osx.tar.gz | tar -xz
```

## 4. Download the Linux build of CS2

Replace `<account>` with your Steam user name. steamcmd asks for the password and the
Steam Guard code in the terminal.

```bash
cd ~/cs2metal-kit/steamcmd
./steamcmd.sh +@sSteamCmdForcePlatformType linux \
  +force_install_dir ~/cs2metal-kit/cs2-linux \
  +login <account> \
  +app_update 730 validate +quit
```

- Expect about 60 GB to download and 71 GB on disk.
- This step is the project's first experiment: nobody is on record using the `linux`
  platform override from a Mac.
- If steamcmd reports that there is no subscription, add `+app_license_request 730`
  before `+app_update` and run it again.

Check the result:

```bash
file ~/cs2metal-kit/cs2-linux/game/bin/linuxsteamrt64/cs2
```

The output must say `ELF 64-bit` and `x86-64`.

## 5. Download the C++ runtime

The Steam Linux Runtime 3.0 ("sniper") is public: no sign-in, 305 MB. The command
extracts only the two libraries the host loads.

```bash
mkdir -p ~/cs2metal-kit/sniper && cd ~/cs2metal-kit/sniper
B=https://repo.steampowered.com/steamrt3/images/latest-public-stable
F='com.valvesoftware.SteamRuntime.Platform-amd64,i386-sniper-runtime.tar.gz'
curl -fL --http1.1 --retry 5 --retry-all-errors -C - -O "$B/$F" \
  && curl -fsSL --retry 5 -O "$B/SHA256SUMS" \
  && grep -F "$F" SHA256SUMS | shasum -a 256 -c - \
  && tar -xzf "$F" --include '*/x86_64-linux-gnu/libstdc++.so.6*' \
                   --include '*/x86_64-linux-gnu/libgcc_s.so.1'
```
The commands are chained, so a failed download stops before the checksum and the
extraction. If the transfer is cut, run the same block again: `-C -` resumes the
partial file instead of starting over.


The `shasum` line must end in `OK`, and `files/lib/x86_64-linux-gnu/` must then hold
`libstdc++.so.6.0.28`, its `libstdc++.so.6` link, and `libgcc_s.so.1`.

## 6. Install the Vulkan SDK and record both drivers

Download the macOS installer from <https://vulkan.lunarg.com/sdk/home> and run it.
Then save what each driver reports on this Mac, from the cs2metal checkout:

```bash
mkdir -p reports
VK_DRIVER_FILES=<MoltenVK ICD manifest from the SDK> \
  vulkaninfo --json -o reports/vulkaninfo-moltenvk.json
VK_DRIVER_FILES=<KosmicKrisp ICD manifest from the SDK> \
  vulkaninfo --json -o reports/vulkaninfo-kosmickrisp.json
```

`VK_DRIVER_FILES` is the Vulkan loader's variable for choosing a driver. The location
of the two manifests depends on where the SDK was installed and has not been verified
on this machine; the SDK's documentation lists it.

## 7. Build cs2metal, record the baseline, and inventory the build

From the cs2metal checkout:

```bash
tools/check.sh
cargo build --release
cargo run --release -p cs2metal -- baseline record ~/cs2metal-kit/cs2-linux
cargo run --release -p cs2metal -- inventory ~/cs2metal-kit/cs2-linux \
  --markdown reports/inventory.md --json reports/inventory.json
cargo run --release -p cs2metal -- inspect ~/cs2metal-kit/cs2-linux/game/bin/linuxsteamrt64/cs2
```

The baseline records the SHA-256 of every game file in the ignored `baselines/`
directory, so that every later step can prove the installation is unchanged
(`baseline verify`). Record it right after the download, before anything else reads
the installation. The inventory lists everything the game expects from the system,
and `inspect` shows what kind of executable `cs2` is. The `reports/` directory is ignored by Git: the
reports are derived from game files and stay local.

## What comes next

With the inventory and Steam in place, three results decide whether the current
design holds, before the loader is written:

| Result | Where it comes from |
| --- | --- |
| The executable type of `cs2`, and the real list of what the host must provide | The `inspect` and `inventory` output of step 7 |
| Whether Steam for macOS serves app 730 | Probe D1 of the [roadmap](ROADMAP.md), designed in the [Steam bridge design](STEAM_BRIDGE_DESIGN.md) |
| Whether a Vulkan driver works in an x86-64 process under Rosetta | Probe F1 of the roadmap, with the reports of step 6 |

Steps 1, 3, 5, and 7 need no credentials and can be run by an automated contributor.
Steps 2 and 4 are the owner's alone. In step 6 the download and installation are the
owner's; the `vulkaninfo` commands can be run by anyone afterwards.
