# The guest's C library

The CS2 Linux build runs on glibc 2.31 of the Steam Linux Runtime. By the
[libc route decision](../../docs/LIBC_ROUTE_DECISION.md), cs2metal gives the guest
that same glibc, rebuilt from Debian's `glibc 2.31-13+deb11u14` source with one
change: every system call goes through one call slot, which the host answers,
instead of the `syscall` instruction, which under Rosetta 2 would reach the macOS
kernel.

```bash
CS2METAL_BUILD_HOST=<ssh destination> tools/guest-glibc.sh
```

| File | Role |
| --- | --- |
| `patch_glibc.py` | The change: 21 sites, the seven `INTERNAL_SYSCALL` templates and `DO_CALL` in `sysdep.h`, twelve lines in nine `.S` files, and `TLS_INIT_TP` in `tls.h`. It fails if the source does not have every site it expects. |
| `Dockerfile` | The pinned Steam Runtime 3 SDK, the fixtures' toolchain, plus `gawk` and `texinfo` |
| `build.sh` | Patches, configures, and builds inside that image without network, then reports, per library, the calls through the slot and the raw `syscall` instructions left |
| `SHA256SUMS` | The checksums of the six libraries; a rebuild is byte-identical |

`tools/guest-glibc.sh` fetches the source once with `apt-get source` inside the SDK
image, so apt checks Debian's signature, and checks both tarballs against pinned
checksums. It then builds and copies the libraries to the ignored
`target/guest-glibc/`. `--update` rewrites `SHA256SUMS` after a deliberate change.

On 27 November 2026 the build took about two minutes on an 8-vCPU virtual machine.
Of 977 system call sites, 974 go through the slot. The 3 left are the
`__restore_rt` signal-return trampolines, which the host never runs because it
delivers signals itself.

## Licence

glibc is LGPL-2.1-or-later. The libraries built here are not tracked. A
distribution of cs2metal that includes them must offer this patch and Debian's
source with them.
