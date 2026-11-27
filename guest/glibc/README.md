# The guest's C library

The CS2 Linux build runs on glibc 2.31 of the Steam Linux Runtime. By the
[libc route decision](../../docs/LIBC_ROUTE_DECISION.md), cs2metal gives the guest
that same glibc, rebuilt from Debian's `glibc 2.31-13+deb11u14` source with one
change: every system call goes through one call slot, which the host answers,
instead of the `syscall` instruction, which under Rosetta 2 would reach the macOS
kernel.

```bash
./build.sh                                                   # builds it only if it is missing
tools/guest-glibc.sh                                         # from source, in Docker on this Mac
CS2METAL_BUILD_HOST=<ssh destination> tools/guest-glibc.sh   # from source, on an x86-64 Linux host with Docker
CS2METAL_GLIBC_FROM=<directory> ./build.sh                   # six libraries built elsewhere, checked
```

| File | Role |
| --- | --- |
| `patch_glibc.py` | The change: 21 sites, the seven `INTERNAL_SYSCALL` templates and `DO_CALL` in `sysdep.h`, twelve lines in nine `.S` files, and `TLS_INIT_TP` in `tls.h`. It fails if the source does not have every site it expects. |
| `Dockerfile` | The pinned Steam Runtime 3 SDK, the fixtures' toolchain, plus `gawk` and `texinfo` |
| `build.sh` | Patches, configures, and builds inside that image without network, then reports, per library, the calls through the slot and the raw `syscall` instructions left |
| `SHA256SUMS` | The checksums of the six libraries. A rebuild is byte-identical, on a Linux host and under emulation on a Mac |

## Why a container, and when it is not needed

glibc builds only with GCC and GNU binutils for Linux; Apple's clang cannot build
it. And the checksums hold for one compiler only: the GCC of the pinned image. So a
build from source runs in that image, an x86-64 Linux container.

That is the one step of cs2metal's build that needs Docker, and it is needed once.
Because the build is reproducible, the six libraries are the same files wherever
they are built, and `SHA256SUMS` says which files they are:

- `./build.sh` first looks for them in `target/guest-glibc/`. If they are there and
  match, nothing is built.
- With `CS2METAL_GLIBC_FROM=<directory>` it takes them from that directory, for
  instance another machine's `target/guest-glibc/`. They are installed only if
  they match.
- Otherwise it builds from source with `tools/guest-glibc.sh`.

The shim libraries of `guest/shims/` need no container. `tools/guest-shims.sh`
builds them on the Mac with Apple's clang and the linker that ships with Rust; they
include no header of a C library.

## The build from source

`tools/guest-glibc.sh` fetches the source once with `apt-get source` inside the SDK
image, so apt checks Debian's signature, and checks both tarballs against pinned
checksums. It then builds without network, copies the libraries to the ignored
`target/guest-glibc/`, and compares them with `SHA256SUMS`. `--update` rewrites
`SHA256SUMS` after a deliberate change to the patch.

On a Mac the image is x86-64 and runs under Docker's emulation. The source and the
build stay in a Docker volume, `cs2metal-glibc-work`, on a Linux file system; only
the finished libraries are copied out. `docker volume rm cs2metal-glibc-work`
removes it.

On 27 November 2026 the build took about two minutes on an 8-vCPU x86-64 virtual
machine and 11 minutes 35 seconds under emulation on an Apple M1 Max, with the same
six checksums. Of 977 system call sites, 974 go through the slot. The 3 left are
the `__restore_rt` signal-return trampolines, which the host never runs because it
delivers signals itself.

## Licence

glibc is LGPL-2.1-or-later. The libraries built here are not tracked. A
distribution of cs2metal that includes them must offer this patch and Debian's
source with them.
