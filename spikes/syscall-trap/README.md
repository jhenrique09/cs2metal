# Syscall trap probe (libc bake-off, route B)

Route B of the libc bake-off (roadmap step C3) would run the Steam Linux Runtime's
real glibc unchanged and translate its system calls. Under Rosetta 2 a Linux
`syscall` instruction runs as a macOS system call with the same number, so every
one would have to be rewritten first. The instruction is 2 bytes and a call is 5,
so the only same-length replacement is a trap (`ud2`, or `int3` plus a byte),
emulated by a signal handler. This probe measures that round trip.

```bash
spikes/syscall-trap/run.sh
```

| Case | What it does |
| --- | --- |
| `int3` | `SIGTRAP` handler sets `rax` and returns |
| `ud2` | `SIGILL` handler sets `rax`, skips the 2-byte trap, and returns |

Each case runs 2000 rounds in its own process with a timeout. A plain function
call is timed alongside for scale.

The results and their consequences are recorded in
[the libc bake-off](../../docs/LIBC_BAKEOFF_2026-11-27.md).
