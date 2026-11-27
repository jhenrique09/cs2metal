# Online play, as the owner reports it, 27 November 2026

## Status

Recorded on 27 November 2026 from what the project owner said after playing. It is
the dated document that [the online launch decision](ONLINE_LAUNCH_DECISION.md) asks
for after the first sessions without `-insecure`. Every result here is **REPORTED**
by the owner: no automated contributor started, watched, or measured an online
session, and none ever does (`AGENTS.md`). The machine is an Apple M1 Max on macOS
27, the game build 26879581. Not edited after the fact.

## Question

Does the game, started without `-insecure`, play on Valve's servers: does it find a
match, connect, and stay in it?

## Method

The owner started the game with `run-online.sh` (at the time
`tools/launch-online.sh`), which is the ordinary launcher without `-insecure`, with
Steam for macOS running and signed in. The owner then used the game's own menus to
find matches, and played them. Nothing else about the launch differs from an offline
one: the installation is verified against its baseline first, and the host applies
the same single in-memory transform, T-001 of the
[architecture](ARCHITECTURE.md).

Nothing about Valve Anti-Cheat was looked at, in these sessions or to write this
document. Rule 4 of the [policy](ANTI_CHEAT_AND_INTEGRITY_POLICY.md) stands.

## Results

- **Casual matches online work.** The owner found, joined, and played casual matches
  against other players, not only against bots. (REPORTED by the owner)
- **Competitive matches online work.** The owner did the same in the competitive
  mode. (REPORTED by the owner)
- **No Valve Anti-Cheat ban so far.** On the day of these sessions the account was
  not banned and no server refused it. (REPORTED by the owner)
- **Everything the offline record lists carries over**: keyboard, mouse, sound, the
  Steam overlay, and the pages the game opens in Steam's browser
  ([the client, playable offline](CLIENT_PLAYABLE_2026-11-27.md)).
- **Stutters at the start of a match.** A casual match stuttered a great deal as it
  began and less as it went on; some rare ones stayed. They are shaders compiled at
  first use and code Rosetta translates at first use, as measured offline.

## What this does not show

- **That it is safe.** One machine and one day say nothing about the next one. How
  Valve Anti-Cheat treats this host is still **UNKNOWN**, and the project does not
  try to find out. Valve Anti-Cheat bans are known to be issued with a delay, so
  "none so far" is not "none". (the delay: REPORTED, Valve's public description of
  its anti-cheat)
- **That a later build of the game or of the host behaves the same.** The host
  changes with every commit, and the game with every update.
- **How many matches, how long, or how steady.** No count, no duration, and no
  pacing figure was taken online. The offline record has the figures there are.

## What stays as it was

- Online play is **at the player's own risk**. Valve does not support this host. The
  loss, if one comes, is the account: play online only with one whose loss would not
  matter (rule 7 of the policy).
- Offline is still the default: `run.sh` passes `-insecure` unless it is told
  `-online`.
- The rules of the policy, all seven.

## Open

- A thirty-minute match with its pacing measured (milestone M8) is still owed, and
  can be taken offline.
- Whether the account stays in good standing is the owner's to report, in a new
  dated document if it changes.
- Voice chat, game controllers, and community servers are not tried.
