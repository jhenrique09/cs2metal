# Steam app probe with the client online, 27 November 2026

## Status

Measured on 27 November 2026 at 07:54 local time. Completes the
[first Steam app probe](STEAM_APP_PROBE_2026-11-27.md) of the same day, which ran
while the client was offline. Closes hypothesis H-2 of the
[Steam bridge design](STEAM_BRIDGE_DESIGN.md). Not edited after the fact.

## Method

Unchanged: `spikes/steam-client/run.sh`, an x86-64 program under Rosetta 2 that
loads the `libsteam_api.dylib` shipped with Steam, declares app 730 through
`SteamAppId`, and asks read-only questions through the flat API. The owner restarted
the Steam client first. Its connection log, read for connection states only, shows a
successful logon at 07:54:04, fourteen seconds before the probe ran.

Steam client: bundle version 6.1. Machine: Apple M1 Max, macOS 27.2.1.

## Results

| Question | Answer |
| --- | --- |
| Does `SteamAPI_InitFlat` accept an x86-64 process declaring app 730? | yes (`OK`) |
| Is the client connected to the Steam servers (`BLoggedOn`)? | **yes**, at the first query |
| Is the signed-in account an individual account in the public universe? | yes |
| Which app does the client see? | 730 |
| Does the account hold app 730 (`BIsSubscribed`, `BIsSubscribedApp(730)`)? | yes, both |

## Interpretation

- **H-2 holds.** The macOS Steam client, connected to the Steam servers, serves the
  Steamworks API to an x86-64 process under Rosetta 2 that declares app 730, and
  answers for the signed-in account and its licence. (CONFIRMED)
- **The bridge design stands as written.** The game's own `libsteam_api.so` will
  reach this same client through the bridged client library. No authentication or
  ownership is emulated anywhere. The answers come from the real client about the
  real account.
- **What this does not show.** Whether the game coordinator, matchmaking, or VAC
  treat a session from this platform differently is outside H-2 and outside the
  first frame. It is observed when it happens, never worked around.
