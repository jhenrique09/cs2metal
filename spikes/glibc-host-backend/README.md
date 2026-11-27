# glibc with a host backend (libc bake-off, route C)

Route C of the libc bake-off (roadmap step C3) builds glibc 2.31, the version the
Steam Linux Runtime ships, from its official source. Every system call goes
through one call slot instead of the `syscall` instruction, so that the host can
answer it and no binary patching of glibc is needed. This spike measures how much
of glibc has to change for that, and whether anything still issues a raw
`syscall`.

```bash
CS2METAL_BUILD_HOST=<ssh destination> spikes/glibc-host-backend/run.sh
```

This spike measures the upstream source; the guest's glibc is built from Debian's
source by `tools/guest-glibc.sh` with the same patch, in `guest/glibc/`.

- `guest/glibc/patch_glibc.py` rewrites the seven `INTERNAL_SYSCALL` assembly templates and the
  `DO_CALL` macro in `sysdeps/unix/sysv/linux/x86_64/sysdep.h`, and every
  hand-written `syscall` in that directory's `.S` files. Each becomes
  `lea -128(%rsp), %rsp; call *%fs:0x20; lea 128(%rsp), %rsp`. The `lea` keeps
  the red zone intact, and `0x20` is `tcbhead_t.sysinfo`, which is unused on
  x86-64; i386 calls the kernel through the same field.
- `guest/glibc/Dockerfile` is the pinned Steam Runtime 3 SDK plus `gawk` and
  `texinfo`.
- `run.sh` downloads the source on the build host and checks its SHA-256, then
  patches, configures, builds without network, and counts the calls through the
  slot and the raw `syscall` instructions left, by function.

The source, the build, and the binaries stay on the build host. The results are
recorded in [the libc bake-off](../../docs/LIBC_BAKEOFF_2026-11-27.md).
