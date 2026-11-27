# Steam client probe (hypothesis H-2)

Answers hypothesis H-2 of the
[Steam bridge design](../../docs/STEAM_BRIDGE_DESIGN.md): will the macOS Steam client
serve app 730 to an x86-64 process under Rosetta 2, although Valve lists no macOS
platform for the app?

```bash
spikes/steam-client/run.sh
```

It needs Steam for macOS running with the owner signed in. The probe loads the
universal `libsteam_api.dylib` that ships inside the Steam installation. It declares
app 730 through `SteamAppId`, the way a game started outside Steam declares
itself, calls `SteamAPI_InitFlat`, and asks read-only questions through the flat API:

| Line | Meaning |
| --- | --- |
| `init` | The result of `SteamAPI_InitFlat`: whether the client accepted the process |
| `connected to Steam` | `ISteamUser::BLoggedOn`: the client's connection to the Steam servers, polled for up to 10 seconds |
| `account` | The type and universe bits of the Steam ID, never the ID itself |
| `app the client sees` | `ISteamUtils::GetAppID` |
| `subscribed to the declared app`, `holds app 730` | `ISteamApps::BIsSubscribed` and `BIsSubscribedApp(730)` |

It prints nothing that identifies the account. It never calls
`SteamAPI_RestartAppIfNecessary`, which could ask Steam to launch the game. While
it runs, Steam may show the account as playing the game for a few seconds.

The results are recorded in
[the Steam app probe report](../../docs/STEAM_APP_PROBE_2026-11-27.md).
