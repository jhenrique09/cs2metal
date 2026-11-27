#!/usr/bin/env bash
# Starts Counter-Strike 2 through cs2metal offline: the same as run.sh -offline.
# The game is started with -insecure, which keeps it off servers secured by
# Valve Anti-Cheat: practice matches with bots, and servers that do not use it.
#
#   run-offline.sh                 start the game, offline
#   run-offline.sh +map de_dust2   extra arguments go to the engine
#
# For online play, and for the environment variables, see run.sh.
set -euo pipefail

exec "$(cd "$(dirname "$0")" && pwd)/run.sh" -offline "$@"
