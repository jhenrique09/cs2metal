# The dedicated server's ten-minute run, 27 November 2026

## Status

Measured on 27 November 2026 on build 26879581 of the CS2 Linux build, for roadmap
step C5. Boots 15 and 16 of the day, run by the agent under the authorization in
`AGENTS.md`: through `cs2metal run`, with `-dedicated -insecure +map de_dust2`,
under timeouts of 120 and 660 seconds. The logs stay in the ignored `runs/`
directory. This document cites only counts, system call numbers, and public
interface names. Not edited after the fact.

## Question

With sockets emulated (commit `1ae77ae`), does the dedicated server load a map and
keep running for ten minutes, the C5 gate?

## Method

```bash
target/x86_64-apple-darwin/release/cs2metal run \
  --game-dir ~/cs2metal-kit/cs2-linux --runtime-dir ~/cs2metal-kit/sniper/files \
  -- -dedicated -insecure +map de_dust2
```

Each boot first checked all 3,131 files of the installation against the baseline,
with no difference. Boot 16 ran a copy of the same binary for 660 seconds, until
its timeout ended it.

## Results

| | Boot 15 (120 s) | Boot 16 (660 s) |
| --- | --- | --- |
| Map loaded, server active | yes | yes |
| Logged on to Steam (anonymous game server, LAN only) | yes | yes, 6.2 s after start |
| Game coordinator session | activated | activated |
| Ran until the timeout | yes | yes, 651 s of engine time |
| Fatal errors, crashes | none | none |
| Unanswered system calls | 330 | 330 |

In both boots the server started its 64-player game, put two bots in, and reported
VAC secure mode disabled, as `-insecure` requires. Its Steam networking
configuration loaded and its game server certificate was issued.

The engine reported 2,976 long frames in boot 16, each between 19 and 22 ms
against the 15.6 ms of a 64-tick server, about 7 percent of its ticks. Missing map
resources (a visibility file and three lightmaps of a prefab) fell back to
defaults, as the engine's own messages describe.

## Interpretation

- **The C5 gate is met.** The dedicated server loads `de_dust2` and keeps running
  for more than ten minutes under cs2metal, under Rosetta 2, with the rebuilt
  glibc, the Steam bridge, and the socket emulation. (CONFIRMED)
- **The whole CPU side holds:** the loader, T-001, threads, futexes, the file
  system through the overlay, physics, networking, and Steam, for 650 seconds, about
  41,000 ticks at 64 a second. (CONFIRMED)
- **The long frames are a lead, not a defect yet.** Each is a few milliseconds
  late, in no visible pattern. Likely causes are the granularity of the host's
  timed waits (`futex`, `epoll_wait`, `clock_nanosleep`) or Rosetta's translation
  of code first reached. (INFERRED; to measure when frame pacing matters, in phase
  F)
- **Memory protection keys (330) stay unsupported**, as before, and nothing
  depends on them.
