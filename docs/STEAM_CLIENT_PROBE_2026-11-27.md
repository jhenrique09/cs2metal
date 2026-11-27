# Steam client probe, 27 November 2026

## Status

Measured on 27 November 2026 on the development machine. Closes hypothesis H-1 of
the [Steam bridge design](STEAM_BRIDGE_DESIGN.md). Not edited after the fact.

## Question

Does the macOS Steam installation contain a client library that an x86-64 process
under Rosetta can load, and does that library export the boundary the design relies
on?

## Method

Steam for macOS was installed by the owner from Valve's disk image and started; no
account was signed in yet. The installed files were inspected read-only with `lipo`
and `nm`. Nothing was loaded, executed, modified, or copied.

Machine: Apple M1 Max, macOS 27.2.1. Steam client build id 1794536281.

## Results

| Check | Result |
| --- | --- |
| Location of the client library | `~/Library/Application Support/Steam/Steam.AppBundle/Steam/Contents/MacOS/steamclient.dylib` |
| Architectures of `steamclient.dylib` | x86-64 and arm64 |
| Architectures of the client executable `steam_osx` | x86-64 and arm64 |
| A `libsteam_api.dylib` inside the installation | Present, in the Steam Helper application, x86-64 and arm64 |
| Exported symbols in the x86-64 slice of `steamclient.dylib` | 38 |
| Boundary functions present | `CreateInterface`, `Steam_BGetCallback`, `Steam_FreeLastCallback`, `Steam_GetAPICallResult`, `Steam_ReleaseThreadLocalMemory`, `Steam_CreateSteamPipe`, `Steam_ConnectToGlobalUser`, `Steam_BLoggedOn` |
| Boundary functions absent | `Steam_IsKnownInterface`, `Steam_NotifyMissingInterface` |

All results are **CONFIRMED** on this machine and client build.

## Interpretation

- **H-1 holds.** The client library has an x86-64 slice, so the host can load it
  directly into its own process. No helper process is needed. (CONFIRMED)
- **The boundary is as small as the design assumed**: 38 exports in all, with the
  factory and the callback polling functions among them. (CONFIRMED)
- **Two functions the Windows `steam_api64.dll` names are not exported here.** They
  let `libsteam_api` ask whether an interface version is known and report a missing
  one. Whether the Linux `libsteam_api.so` of the game imports them is UNKNOWN until
  the inventory runs. If it does, the bridge must answer them itself: by asking the
  factory for the interface and reporting whether it exists, which fabricates
  nothing. Probe H-3 of the design changes accordingly.
- **The installation carries its own universal `libsteam_api.dylib`.** That makes the
  design's fallback option C possible without any outside download, though its
  version is the client's, not the game's. (CONFIRMED presence; INFERRED use.)

## Reproduce

```bash
S=~/Library/Application\ Support/Steam/Steam.AppBundle/Steam/Contents/MacOS
lipo -info "$S/steamclient.dylib"
nm -gU -arch x86_64 "$S/steamclient.dylib" | wc -l
```
