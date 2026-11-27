# Security

## Scope

cs2metal loads third-party x86-64 code into its own process: the game's, and the
C++ runtime's. Treat every game file as untrusted input: the ELF parser is
bounds-checked and must never panic, and the loader validates every address it
derives from a file before using it. The guest code itself is not sandboxed: it
runs with the rights of the user who starts it, as the game would on Linux.

## What the scripts download and run

- `install.sh` downloads steamcmd and the Steam Linux Runtime from Valve's servers
  over HTTPS, and checks the runtime against the checksum list published beside it.
  The game is downloaded by steamcmd, which validates every file.
- `tools/guest-glibc.sh` fetches Debian's glibc source with apt, which checks
  Debian's signature, and checks the tarballs against checksums pinned in the
  script. The container image is pinned by digest. The build runs without network.
- `build.sh` installs the guest's C library only if its six files match the
  checksums in `guest/glibc/SHA256SUMS`, wherever they come from.
- Before every start, cs2metal verifies each game file against the recorded
  baseline and refuses to start on a difference.

## Secrets

The project never asks for, stores, or forwards Steam credentials. Steam sign-in
happens only in Valve's own programs: the Steam client, and steamcmd if you choose
to download as your account instead of anonymously. If a secret is committed by
mistake, rotate it first, then remove it from history.

## Reporting

Report a vulnerability privately to the repository owner instead of opening a public
issue. Include the affected commit, the input that triggers the problem, and the
observed behavior.

Questions about Valve Anti-Cheat are not security reports for this project: it does
not study it, by `docs/ANTI_CHEAT_AND_INTEGRITY_POLICY.md`.
