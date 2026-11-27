# Decision: an opt-in online launch

## Status

Accepted on 27 November 2026 by the project owner, who asked for the launcher option
after reading the risks listed under Context. It is the dated decision record that
rule 6 of the [policy](ANTI_CHEAT_AND_INTEGRITY_POLICY.md) asks for.

## Context

Rule 6 keeps every launch offline: the game starts with `-insecure`, which keeps it
off servers secured by Valve Anti-Cheat. That covers matchmaking and most community
servers. Since 27 November 2026 the client is playable offline
([record](CLIENT_PLAYABLE_2026-11-27.md)), and the owner wants to be able to start it
without that option.

What is known about the risk, as told to the owner before the decision:

- The host changes guest code in memory: transform T-001 of the
  [architecture](ARCHITECTURE.md) rewrites `fs` prefixes to `gs` in every executable
  module. Files on disk stay untouched. Valve names modifications to a game's
  executables and libraries as cheating (policy, "What Valve says").
- How Valve Anti-Cheat treats this host is **UNKNOWN**, and rule 4 forbids finding
  out by studying it. The outcome of a session on a secured server can range from a
  refused connection to a permanent ban of the account.
- The risk does not shrink as the host matures: T-001 is structural. Only the choice
  of account limits the loss.
- The client is one day old: one boot aborted on events taken off the main thread,
  and sound under a match's load is not measured. A crash in a competitive match
  counts as abandoning it.

## Decision

`tools/launch.sh` accepts `-online`, and `tools/launch-online.sh` is that launcher
with the option already given. With it the launcher does not pass `-insecure`;
nothing else about the launch changes, and the installation is still verified
against its baseline first. Without it the launcher passes `-insecure`, as before.
The option is the launcher's own and does not reach the engine.

Offline stays the default. Going online is a choice the owner makes at each launch.

## What does not change

- Rules 1 to 5 of the policy, and rule 7: a secondary account for anything online.
- Automated contributors boot the game only through `cs2metal run` with `-insecure`
  and never pass `-online`; matchmaking and secured servers stay owner actions
  (`AGENTS.md`).
- Nothing about Valve Anti-Cheat is studied, intercepted, or worked around. If a
  secured server refuses the session, that is recorded as observed and left alone.

## Consequences

- The first session without `-insecure` is an owner action. What it shows goes into
  a dated document, with only what the policy allows from the log.
- The untested step between offline play and secured servers remains available: a
  client with `-insecure` joining a server that does not use Valve Anti-Cheat, such
  as the dedicated server cs2metal already runs.
