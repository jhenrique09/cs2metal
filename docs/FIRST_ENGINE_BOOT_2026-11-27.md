# The first engine boot under cs2metal, 27 November 2026

## Status

Measured on 27 November 2026 on build 26879581 of the CS2 Linux build, for roadmap
steps C4 and C5. The first boot was the owner's; the second ran under the
authorization recorded in `AGENTS.md`, through `cs2metal run` with
`-dedicated -insecure`, not signed in to Steam, under a 60-second timeout. The
logs stay in the ignored `runs/` directory. This document paraphrases them and
cites only counts, system call numbers, and public interface names. Not edited after
the fact.

## Question

Does the engine boot under cs2metal, the rebuilt glibc 2.31, and the Steam Linux
Runtime's libraries, as far as its Steam initialization and its first SDL or
Vulkan call?

## Method

```bash
target/x86_64-apple-darwin/release/cs2metal run \
  --game-dir ~/cs2metal-kit/cs2-linux --runtime-dir ~/cs2metal-kit/sniper/files \
  -- -dedicated -insecure +map de_dust2
```

`cs2metal run` verified all 3,131 files of the installation against the baseline,
with no difference, before starting the engine. Unsupported system calls are named
once on standard error.

## Results

| Boot | Where it stopped | System calls unanswered |
| --- | --- | --- |
| 1 (owner) | Creating an event: `eventfd2` was unanswered (Linux error 38, `ENOSYS`), after the engine had loaded `libSDL3.so.0` | 201, 99, 290, 22 |
| 2 (agent), after commit `0748b6f` | `SteamGameServer_Init` (public Steamworks API) failed to load the Steam client library from `~/.steam/sdk64/steamclient.so`, a fatal error by the engine's own rule | 37, 143, 145, 221, 291, 330, none fatal |

In boot 2, in order, the engine:

- loaded its engine module and `libSDL3.so.0` through `dlopen`, set its Steam app
  id to 730, and initialized its crash reporting;
- loaded 25 modules through `dlopen` in all (`libtier0`, `libfilesystem_stdio`,
  `librendersystemempty`, the scene, physics, sound, network, animation, and server
  modules). Four optional ones were absent, as they are on Linux without them,
  `librenderdoc.so` among them;
- mounted its search paths and VPK archives;
- reported its server module initialized about 15 seconds after start, finished
  its Source 2 initialization, and began loading `de_dust2`;
- called `SteamGameServer_Init`, which stopped the boot.

One warning was not fatal: `setlocale` for `en_US.UTF-8` failed, because the guest
has no locale data, and the engine used the `C` locale.

## Interpretation

- **The C4 gate is met.** The engine's own log shows its Steam initialization
  attempt and its first SDL use: the `dlopen` of `libSDL3.so.0`, after which the
  engine went on. (CONFIRMED) The authorization for agent-run boots ends here, as
  recorded.
- **The route holds on the real game.** The rebuilt glibc, T-001 at every
  executable mapping of the build's 61 modules and the runtime's, the bridge, and
  threads carried the engine through its initialization and into level loading,
  under Rosetta 2. (CONFIRMED)
- **What stops it now is the Steam bridge, roadmap step D2.** The game server's
  Steamworks initialization looks for the Linux Steam client library where the
  Linux Steam client installs it. It finds nothing there, which on Linux too is
  fatal for a dedicated server. The [Steam bridge design](STEAM_BRIDGE_DESIGN.md)
  answers that path with the macOS client library. For a game server, though, the
  client logs on to Steam's servers anonymously, which is outside the recorded
  authorization. (CONFIRMED stop; INFERRED next behavior)
- **The six unanswered calls were tolerated.** `epoll` (291), which networking will
  need, is now emulated on kqueue. Alarm, scheduling queries, and file advice are
  answered. Memory protection keys (330) stay unsupported, which V8 accepts.
