#!/usr/bin/env bash
# Starts Counter-Strike 2 through cs2metal WITHOUT -insecure: the same as
# run.sh -online. VAC-secured servers become reachable, and how Valve Anti-Cheat
# treats this host is unknown, so the account signed in to Steam carries the
# risk. It is the owner's opt-in, recorded in docs/ONLINE_LAUNCH_DECISION.md: use
# a secondary account. Automated contributors never run this.
#
#   run-online.sh                 start the game, online
#   run-online.sh +map de_dust2   extra arguments go to the engine
#
# For offline play, and for the environment variables, see run.sh.
set -euo pipefail

exec "$(cd "$(dirname "$0")" && pwd)/run.sh" -online "$@"
