# The first boots through the Steam bridge, 27 November 2026

## Status

Measured on 27 November 2026 on build 26879581 of the CS2 Linux build, for roadmap
step D2. Boots 12 to 14 of the day, run by the agent under the authorization in
`AGENTS.md`: through `cs2metal run`, with `-dedicated -insecure +map de_dust2`, under
a timeout, with the macOS Steam client signed in. The logs stay in the ignored
`runs/` directory. This document cites only counts, system call numbers, and public
interface names. Not edited after the fact.

## Question

Does the game's own `libsteam_api.so`, unmodified, initialize a game server through
the bridged client library and the owner's macOS Steam client?

## Method

The bridge as the [Steam bridge design](STEAM_BRIDGE_DESIGN.md) describes it:

- `~/.steam/sdk64/steamclient.so` is answered with cs2metal's shim, built with the
  guest glibc. Nothing is written to the owner's home.
- The shim's constructor makes the private system call `0x43530001`. The host loads
  the macOS `steamclient.dylib` into its own domain and fills the shim's table with
  fixing thunks into it, by name. The shim's export list is generated at build time
  from the owner's library, by `tools/steam-shim-names.sh`, and is never tracked.
- Whatever a client call returns that is one of the library's objects (its vtable
  lies inside the library) reaches the guest as a proxy, whose 256 slots cross the
  bridge in turn.
- The app id the game writes to `steam_appid.txt` (through the installation overlay)
  is mirrored into the host environment, where the macOS library looks for it.
- `CS2METAL_STEAM_TRACE=1` logs every bridged call: the exported name, or the
  proxied object and vtable slot it reached.

## Results

| Boot | Where it stopped |
| --- | --- |
| 12 | `SteamGameServer_Init` failed for want of the `SteamUtils010` interface. The trace shows the factory making two client interfaces and `CreateLocalUser` returning user 1, then the host's `Steam_IsKnownInterface` answering no for `SteamUtils010` |
| 13 | The same (the traced build had gone to the wrong target directory) |
| 14 | **`SteamGameServer_Init` succeeded** and the server began logging on to Steam anonymously (no game server token, so LAN only, as on Linux). `SteamGameServerNetworking` succeeded too. Then the engine stopped, fatally, in `SteamDatagramServer_Init`, because `socketpair` (system call 53) was unanswered |

In boot 14 the game's library asked the factory for three `SteamClient` versions
(017, 020, 023); the macOS library made all three.

## Interpretation

- **The macOS library does not export `Steam_IsKnownInterface`**, as the
  [client probe](STEAM_CLIENT_PROBE_2026-11-27.md) found. The game's library asks it
  about every interface version before taking it from the client. Most versions
  (`SteamUtils`, `SteamGameServer`, and so on) come from `ISteamClient` methods, not
  from the factory, so asking the factory was wrong. The host now answers yes when
  the library names the version among the C strings of its `__TEXT,__cstring`
  section, which is where every version it hands out is named. (CONFIRMED by boot 14)
- **D2's dedicated-server half works.** The game's unmodified Steam API library
  initialized a game server through the owner's macOS client and began its anonymous
  logon. No identity, ownership, or session is emulated: every call reached the real
  client. (CONFIRMED)
- **What stops the boot now is networking.** The engine needs sockets, which the
  guest did not have at all. That is the next slice.
