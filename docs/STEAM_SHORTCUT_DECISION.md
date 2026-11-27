# Decision: the shortcut may be added to Steam's library

## Status

Accepted on 27 November 2026 by the project owner, who asked for a script that makes
a shortcut for the game and adds it to Steam.

## Context

The [Steam bridge design](STEAM_BRIDGE_DESIGN.md) says cs2metal does not modify any
Steam file. Steam offers no command or URL that adds a non-Steam game: its client
keeps the list, per account, in `userdata/<account>/config/shortcuts.vdf`, and adds
to it only from its own window.

## Decision

`create-shortcut.sh --steam` adds one entry, for the application it has just made,
to that file for every account on the Mac. It is the one place where cs2metal
writes a file of Steam's, and it is bounded:

- Only when the owner passes `--steam`, or answers yes to `install.sh`'s question.
- Only while Steam is not running, because Steam writes the file when it quits. The
  script refuses otherwise and never quits or starts Steam itself.
- The file is read and written whole, in its own format, by
  `tools/steam_shortcuts.py`; a file it cannot read completely is left untouched.
- The file as it was is copied once to `shortcuts.vdf.before-cs2metal` beside it.
- Nothing else of Steam's is read or written: no session, credential, or
  configuration file.

## Consequences

- The game can be started from Steam's library, which starts the application, which
  runs `run.sh`. The game still declares app 730 to Steam through the bridge, as
  when started from a terminal.
- Not yet tried on the owner's Steam: how Steam shows the running shortcut beside
  app 730, and whether its own overlay injection matters, are unmeasured.
- Removing the entry is done in Steam's library, or by restoring the copy.
