# Steam app probe, 27 November 2026

## Status

Measured on 27 November 2026 at 07:45 local time on the development machine, for
hypothesis H-2 of the [Steam bridge design](STEAM_BRIDGE_DESIGN.md) and roadmap step
D1. **Partial**: the macOS client was signed in but had no connection to the Steam
servers, so the online half of H-2 must be measured again. Not edited after the
fact; the second run gets its own document.

## Question

Will the macOS Steam client serve app 730 to an x86-64 process under Rosetta 2,
although Valve lists no macOS platform for the app?

## Method

The probe `spikes/steam-client/` in the repository is an x86-64 program run under
Rosetta 2. It loads the universal `libsteam_api.dylib` that ships inside the Steam
installation, declares app 730 through the `SteamAppId` environment variable,
calls `SteamAPI_InitFlat`, and asks read-only questions through the flat API. It
prints nothing that identifies the account and does not ask Steam to launch
anything. The client's own connection log was read afterwards, for its connection
states only.

Steam client: bundle version 6.1, build id 1794536281. Machine: Apple M1 Max,
macOS 27.2.1.

## Results

| Question | Answer |
| --- | --- |
| Does `SteamAPI_InitFlat` accept an x86-64 process declaring app 730? | **yes** (`OK`) |
| Which app does the client see? | **730** |
| Is the signed-in account an individual account in the public universe? | yes |
| Does the account hold app 730 (`BIsSubscribedApp(730)`, `BIsSubscribed`)? | **yes**, both |
| Is the client connected to the Steam servers (`BLoggedOn`), within 10 s? | **no** |

The connection log explains the last row. The client's last successful logon was at
01:09 that day and was followed by a log-off. Every reconnection attempt since then
was answered "Try another CM" and the remote host closed the connection. That
happened every 9 to 21 minutes, before and after the probe ran. The client was
running offline with the account remembered. The probe did not cause it.

## Interpretation

- **The platform question of H-2 is answered: the macOS client does not refuse app
  730.** It initializes the Steamworks API for an x86-64 process that declares the
  app, identifies the app, and answers ownership from the account's licences.
  (CONFIRMED) The fallback that H-2's failure would have triggered does not apply.
- **The online half is still open.** Whether the servers accept the client while it
  runs app 730, which matters for the game's own logon and later for matchmaking,
  needs a run with the client connected. (UNKNOWN)
- **Ownership comes from the client, as the policy requires.** Nothing was emulated:
  the answers are the real client's, about the real account.

## Next

Bring the Steam client back online, by restarting it or checking the network, so
that its connection log shows a logon. Then run `spikes/steam-client/run.sh` again
and record the result in a new dated document. H-2 passes when `connected to
Steam` is `yes` with app 730 seen and held.
