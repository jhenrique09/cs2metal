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

## Supported versions

cs2metal has no releases yet. Only the latest commit on `main` is supported: fixes
land there, and a report against an older commit is checked against `main` first.

## Reporting a vulnerability

**Do not open a public issue.** Report it privately, through GitHub:
[open a private security advisory](https://github.com/jhenrique09/cs2metal/security/advisories/new).
Only the maintainer sees it.

Please include:

- the affected commit (`git rev-parse --short HEAD`);
- what an attacker controls: a file, a game update, a network peer, another local
  program;
- the input or the steps that trigger it, and what you observed;
- the impact as you understand it.

Leave out your Steam credentials and session files, and remove your account name
and Steam ID from any log.

## What to expect

- An acknowledgement within seven days.
- An assessment, and a plan or a fix, within thirty days for a confirmed problem.
- Credit in the advisory and the fixing commit, unless you would rather stay
  anonymous.
- Disclosure once a fix is on `main`, coordinated with you. If a fix takes longer,
  we agree on a date together.

## In scope

- Memory safety of the host when it parses or maps files: the ELF parser, the
  loader, the integrity baseline.
- The system call layer and the bridges, when guest input can make the host act
  outside what a Linux kernel would allow the same process.
- The scripts: anything that downloads, installs, or runs code without checking it,
  or writes outside `~/cs2-metal` and the files they document.
- Anything that could expose a Steam credential or session file.

## Out of scope

- The game itself, Steam, steamcmd, MoltenVK, SDL, and glibc: report those to Valve,
  the Khronos Group, the SDL project, or Debian.
- That guest code runs with the user's rights. It is by design, as on Linux.
- Valve Anti-Cheat, bans, and accounts. The project does not study anti-cheat, by
  `docs/ANTI_CHEAT_AND_INTEGRITY_POLICY.md`, and cannot help with them.

## Safe harbor

Research done in good faith, on your own machine and your own account, within this
policy, is welcome: the project will not pursue it. That promise covers this
project only. It cannot speak for Valve, whose terms still govern the game and
Steam.
