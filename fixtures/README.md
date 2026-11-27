# Linux fixtures

Small Linux programs written in this project: the loader's test suite (roadmap
step C1). They exercise, one at a time, what the CS2 build needs from a Linux
process, in the toolchain and libraries it targets: the Steam Runtime 3 ("sniper")
SDK, gcc 10.3, and glibc 2.31.

| Fixture | Files | What it exercises |
| --- | --- | --- |
| `hello` | `hello` | libc output, arguments, environment, exit status |
| `tls` | `tls`, `libtls_lib.so` | `__thread` in the executable (local-exec through `fs`) and in a library (global-dynamic through `__tls_get_addr`), four threads |
| `stack_guard` | `stack_guard` | the stack protector's canary at `fs:0x28`, in every function |
| `cxx` | `cxx`, `libcxx_lib.so` | C++ exceptions thrown in a library and caught in the executable, iostream, static constructors, the runtime's `libstdc++` |
| `dlopen` | `dlopen`, `libdlopen_plugin.so` | `dlopen`, `dlsym`, `dlerror`, `dlclose`, a constructor, `$ORIGIN` |
| `ifunc` | `ifunc` | an IFUNC resolver in the executable, and glibc's own IFUNC string functions |
| `clock` | `clock` | `clock_gettime`, `gettimeofday`, and `time` through the vDSO, and the sleeps and condition waits whose deadlines are computed from them |
| `scope` | `scope`, `libscope_user.so`, `libscope_over.so`, `libscope_base.so` | `DT_NEEDED` through `$ORIGIN`, symbol versions (`versioned@SCOPE_1`, `versioned@@SCOPE_2`), a weak definition overridden, a call interposed inside its own module; the libraries need no C library |

## Build and check

```bash
CS2METAL_BUILD_HOST=<ssh destination> tools/fixtures.sh   # on an x86-64 Linux host
tools/fixtures.sh                                         # local Docker, emulated
```

The script builds `src/` with `build.sh` inside the SDK image, pinned by digest,
without network. It then runs every fixture natively with `run.sh`, in the same
image, and compares the output and exit status with `expected/`. Last, it copies
the binaries to `target/fixtures/` and checks them against `SHA256SUMS`.
`--update` rewrites `expected/` and `SHA256SUMS` after a deliberate change.

`CS2METAL_BUILD_HOST` is an ssh destination, ideally an alias from `~/.ssh/config`,
for an x86-64 Linux machine with Docker. It never goes into the repository. The
script says which path it takes. On 27 November 2026 both ran, a Debian 13 x86-64
virtual machine natively and OrbStack's Docker emulating linux/amd64 on the Mac.
Their builds are byte-identical, and every fixture gives its expected output on
both.

## Why the binaries are not tracked

The binaries are rebuilt, never committed, by the
[fixtures decision](../docs/FIXTURES_DECISION.md). The repository keeps the
sources, the expected outputs, and the checksums. The flags avoid anything
machine- or time-specific, so a rebuild is byte-identical to `SHA256SUMS`. That
proves the binaries in `target/fixtures/` come from these sources.
