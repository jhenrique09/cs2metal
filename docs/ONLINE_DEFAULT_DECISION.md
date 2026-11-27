# Decision: the launcher starts online by default

## Status

Accepted on 27 November 2026 by the project owner, after playing casual and
competitive matches online ([record](ONLINE_PLAY_2026-11-27.md)). It replaces the
default set by [the opt-in online launch](ONLINE_LAUNCH_DECISION.md), and is the
dated decision record that rule 6 of the
[policy](ANTI_CHEAT_AND_INTEGRITY_POLICY.md) asks for.

## Context

Until this decision `run.sh` passed `-insecure` unless it was told `-online`, and
`run-online.sh` was the launcher with that option given. The owner played online
with it the same day and asked that online be what the launcher does when it is
told nothing: it is how the game is meant to be played, and how Steam starts it.

What is known about the risk has not changed since the earlier decision:

- The host changes guest code in memory: transform T-001 of the
  [architecture](ARCHITECTURE.md). Files on disk stay untouched. Valve names
  modifications to a game's executables and libraries as cheating (policy, "What
  Valve says").
- How Valve Anti-Cheat treats this host is **UNKNOWN**, and rule 4 forbids finding
  out by studying it. The owner's sessions of one day ended with no ban; a ban can
  come later than the session that caused it.
- The loss, if one comes, is the Steam account that was signed in.

A default is what a person gets without having chosen. So this decision moves the
risk from those who ask for it to everyone who starts the game, and the launcher
and the documentation have to say so where it cannot be missed.

## Decision

`run.sh` starts the game without `-insecure`. `run.sh -offline`, and
`run-offline.sh`, which is that launcher with the option given, pass `-insecure`:
practice matches with bots, and servers that do not use Valve Anti-Cheat. The
option is the launcher's own and does not reach the engine. `-online` is still
accepted and changes nothing.

The application `create-shortcut.sh` makes runs `run.sh`, so it starts online too;
`create-shortcut.sh --offline` makes a second one that starts offline.

Each online start says, in the terminal, that secured servers are reachable at the
player's own risk and how to start offline. `install.sh` says the same as it ends,
and the README opens with it.

## What does not change

- Rules 1 to 5 of the policy, and rule 7: play online only with an account whose
  loss would not matter.
- Automated contributors never start the game without `-insecure`: they boot it
  through `cs2metal run` with `-insecure`, or through `run.sh -offline`, and never
  through `run.sh` alone (`AGENTS.md`).
- Nothing about Valve Anti-Cheat is studied, intercepted, or worked around. If a
  secured server refuses the session, or an account is banned, that is recorded as
  reported and left alone.
- The installation is verified against its baseline before every start, online or
  offline.

## Consequences

- Anyone who follows the README plays online, on their own account, unless they
  choose otherwise. The project cannot tell them that this is safe, and does not.
- A crash of the host in a competitive match counts as abandoning the match.
- If the owner, or anyone, reports a ban or a refused session, it goes into a new
  dated document, and this decision is reviewed.
