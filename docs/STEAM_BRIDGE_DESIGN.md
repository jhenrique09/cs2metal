# Steam bridge design

## Status

Accepted on 27 November 2026, when probes H-1 and H-2 below passed against the
owner's Steam client ([client library probe](STEAM_CLIENT_PROBE_2026-11-27.md),
[Steam app probe online](STEAM_APP_PROBE_ONLINE_2026-11-27.md)). Nothing here is implemented yet; it is the plan of
record for milestone M7 of the [implementation plan](IMPLEMENTATION_PLAN.md).
H-3 to H-5 are measured as the bridge is built.

## Question

The Linux build of CS2 expects a Linux Steam client. On a Mac only the macOS Steam
client exists. How does the game reach Steam without the project emulating
authentication, ownership, or any other part of Steam, as rule 5 of the
[policy](ANTI_CHEAT_AND_INTEGRITY_POLICY.md) forbids?

## How Steamworks is layered

Three pieces sit between a game and Valve's servers (Steamworks documentation,
REPORTED):

```
game modules
  └─ libsteam_api        thin library shipped WITH THE GAME: finds the client,
       │                 hands out interface pointers, dispatches callbacks
       └─ steamclient    library shipped WITH THE STEAM CLIENT: implements every
            │            interface and talks to the client process
            └─ Steam client process: the signed-in account, licences, sessions
```

Identity, licences, and sessions live in the Steam client process. Neither library
in the game's process holds credentials. The boundary between `libsteam_api` and
`steamclient` is narrow: one factory function (`CreateInterface`), a few plain C
functions for callback polling (`Steam_BGetCallback`, `Steam_FreeLastCallback`,
`Steam_GetAPICallResult`, and a handful more), and C++ interface objects obtained
through the factory. (REPORTED: the public strings of the Windows `steam_api64.dll`
in SteamDB's [GameTracking-CS2](https://github.com/SteamDatabase/GameTracking-CS2)
name these functions; the Linux library is INFERRED to match.)

## Options

| Option | Summary | Verdict |
| --- | --- | --- |
| A. A stand-in Steam API that answers by itself | What "Steam emulators" do | **Forbidden.** It fabricates identity and ownership. |
| B. A Linux Steam client | Run Valve's Linux client next to the game | Impossible without a Linux kernel; it would mean a virtual machine. |
| C. Replace the game's `libsteam_api.so` with a macOS `libsteam_api.dylib` | Bind the game's Steam imports to the macOS library | Possible fallback. The boundary is the whole flat API, the host library would call guest callback objects directly, and the project would need a macOS library that matches the game's SDK version. |
| D. **Keep the game's own `libsteam_api.so`; give it the macOS `steamclient`** | Load the Linux `libsteam_api.so` as guest code, unmodified. When it asks for the client library, hand it a bridge to the `steamclient.dylib` inside the owner's macOS Steam installation. | **Proposed.** Narrowest boundary, and every byte of Steam logic that runs is Valve's own. |

## Proposed design (option D)

### 1. Declaring the app

A game started by Steam receives its app id in the environment (`SteamAppId`,
`SteamGameId`). The host sets the same variables to `730` for the guest. This is the
mechanism Valve documents for running a game outside the client during development,
whose file form is `steam_appid.txt`. The host uses the environment form only: the
file form would add a file to the installation, which the integrity rule forbids.

Setting the app id asserts nothing about ownership. The Steam client decides, from
the signed-in account's own licences, whether to serve app 730.

### 2. Finding the client

The Linux `libsteam_api.so` looks for a running client and for the client library
under `~/.steam`; one public report shows it opening `~/.steam/sdk64/steamclient.so`
(REPORTED, single source; the exact sequence is UNKNOWN until observed). The glibc
ABI layer already mediates every file and process call of the guest, so it answers
these lookups from the real state of the Mac:

- Whether Steam is running is the truth about the macOS Steam process, with its real
  process id. If Steam is not running, the guest sees exactly that.
  Built on 27 November 2026. Observed: the public `SteamAPI_IsSteamRunning` reads a
  process id from `~/.steam/steam.pid` and asks, with `kill` and signal 0, whether
  that process lives. A read of that path is answered with a file, written outside
  the owner's home, that holds the id of the running macOS Steam client, found by
  its executable; while the client is not running there is no file. In client boot
  49 the game's Steam API no longer reported that it found no running Steam.
- The path of the client library resolves to a name the loader recognizes.

Nothing is written to disk and no Steam file is touched.

### 3. Loading the client library

When the guest calls `dlopen` on the client library, the loader returns a **bridged
module**: its symbols are the exports of the macOS `steamclient.dylib`, each reached
through a host thunk that switches the thread to the host domain for the call (see
"Execution domains and the bridge" in [ARCHITECTURE.md](ARCHITECTURE.md)). The
library is loaded from the owner's Steam installation at run time and is never
copied or redistributed.

### 4. Interface objects

`CreateInterface` and the getters of `ISteamClient` return C++ objects that live in
host code. Both platforms use the Itanium C++ ABI, so their virtual tables have the
layout the guest expects, but a guest call through such a table would run macOS code
in the guest domain. Every interface pointer that crosses to the guest is therefore
replaced by a **proxy** whose virtual table holds generic thunks:

- A thunk switches to the host domain, calls the same slot of the real object, and
  switches back on return. It keeps every argument register and the stack untouched
  and keeps the return address on a per-thread shadow stack, so it needs no
  knowledge of any method's signature.
- The set of calls that return interfaces is small and public: the factory and the
  `ISteamClient` getters, in the order of the public Steamworks headers.

### 5. Callbacks

Callback dispatch stays in guest code. The guest `libsteam_api.so` polls the client
through the plain C functions of the boundary and then calls the game's callback
objects itself, guest to guest. No host code calls a guest object on this path.

The exceptions are the few interfaces that take a caller-implemented object or a
function pointer (the warning message hook, server list responses, the networking
sockets callbacks). Each needs a reverse thunk that enters the guest domain. None is
expected before the first frame; they are listed as later work, one by one, from the
public headers.

### 6. Data layouts

Structures passed across the boundary must have the same layout on both sides. The
public Steamworks headers select the same structure packing for Linux and macOS, and
both sides use the same C++ ABI. (INFERRED from the public headers; verified by H-4.)

## What this design does and does not do

| Does | Does not |
| --- | --- |
| Run the game's own, unmodified `libsteam_api.so` | Fabricate any answer about identity, licences, or sessions |
| Call Valve's own `steamclient.dylib` from the owner's installation | Read, store, or forward credentials, tokens, or session files |
| Declare app 730 the way Steam's launcher and Valve's documentation do | Modify, copy, or redistribute any Steam file |
| Report "Steam is not running" or a refused app truthfully and stop | Start, script, or automate the Steam client or its sign-in |
| Launch with `-insecure` by default, keeping the game off VAC-secured servers | Touch Valve Anti-Cheat in any way |

Consequences the owner should expect: while the game runs, the account shows as
playing Counter-Strike 2, friends can see it, and play time accrues. That is the real
client doing its job. The Steam overlay was not injected at first, because nothing preloaded it. Since 27
November 2026 the launcher (`tools/launch.sh`) inserts the macOS client's own
overlay renderer, `gameoverlayrenderer.dylib`, into the host process with
`DYLD_INSERT_LIBRARIES`, and sets `SteamAppId` and `SteamGameId`, as Steam does for a
game it starts. The renderer hooks the Metal layer MoltenVK presents on, and Steam
then starts its overlay process for the game: the owner opened the overlay with
Shift+Tab in client boot 67. It is Valve's library from the owner's installation,
loaded from where it is; `CS2METAL_NO_OVERLAY=1` leaves it out.

## Hypotheses and the probes that test them

In order; each probe is small and isolated, and the first two need no guest code.

| Id | Hypothesis | Probe | If false |
| --- | --- | --- | --- |
| H-1 | The macOS Steam installation contains a `steamclient.dylib` with an x86-64 slice. | `lipo -info` on the library after installing Steam. **CONFIRMED on 27 November 2026** ([probe](STEAM_CLIENT_PROBE_2026-11-27.md)). | The bridge would need a helper process of the library's architecture. |
| H-2 | An x86-64 macOS process that declares app 730 can connect to the running client and query the signed-in user and the app's licence, although Valve lists no macOS platform for the app. | A native probe under `spikes/` that loads the library, creates a pipe, connects to the global user, and asks `ISteamUser` and `ISteamApps` two read-only questions. | Steam on macOS refuses the app; the bridge cannot work as designed and the fallback below applies. |
| H-3 | The interface versions the game's `libsteam_api.so` requests are known to the macOS client library. | Ask the factory for each version string the inventory finds. (The macOS library does not export `Steam_IsKnownInterface`; see the [probe](STEAM_CLIENT_PROBE_2026-11-27.md).) | Per-interface gaps; the game normally tolerates a newer client. |
| H-4 | Callback and result structures have identical layouts. | Compare sizes and offsets from the public headers compiled for both targets. | A translation table for the structures that differ. |
| H-5 | The engine needs Steam before it creates its window. | The first boot log of M4: the engine logs the outcome of its Steam initialization either way. (REPORTED from public strings.) | If initialization failure is not fatal, the first frame does not depend on this bridge at all. |

## Risks

| Risk | Mitigation |
| --- | --- |
| The macOS client refuses app 730 on platform grounds (H-2). | Measure it first, with a native probe, before building anything else of the bridge. |
| Valve ships the client library without an x86-64 slice once Rosetta is gone. | Same horizon as the host itself; decided at milestone M9. |
| The game updates weekly and its `libsteam_api.so` moves ahead of the installed client. | The same skew exists on every platform; interfaces are versioned by name. Keep Steam updated. |
| The main menu needs the game coordinator, which may treat an unknown platform differently. | Out of scope for the first frame; observed, never worked around. |

## Fallback

If H-2 fails, option C does not help, because it reaches the same client. The honest
fallback is to boot as far as the engine goes without Steam (H-5) and to record the
limit. Option A stays forbidden in every case.
