#!/bin/sh
# Builds every fixture inside the Steam Runtime 3 (sniper) SDK image, the toolchain
# and libraries the CS2 Linux build targets. Run by tools/fixtures.sh with the
# fixture tree mounted read-only at /src and an empty output directory at /out.
# The flags avoid anything machine- or time-specific, so the outputs are
# byte-identical on every build and their checksums can be tracked.
set -eu

src=/src/src
out=/out
cc="gcc"
cxx="g++"
common="-O2 -g0 -fPIC -Wall -Werror -ffile-prefix-map=/src=. -fno-ident"
link="-Wl,--hash-style=gnu -Wl,-z,now -Wl,--build-id=sha1"
origin="-Wl,-rpath,\$ORIGIN"

mkdir -p "$out"
cd "$out"

# hello: libc only.
$cc $common $link "$src/hello.c" -o hello

# tls: local-exec TLS in the executable, global-dynamic TLS in a library, threads.
$cc $common $link -shared "$src/tls_lib.c" -o libtls_lib.so
$cc $common $link -pthread "$src/tls_main.c" -L. -ltls_lib $origin -o tls

# stack_guard: every function reads the canary at fs:0x28.
$cc $common $link -fstack-protector-all "$src/stack_guard.c" -o stack_guard

# cxx: exceptions across a shared object, iostream, static constructors.
$cxx $common $link -shared "$src/cxx_lib.cpp" -o libcxx_lib.so
$cxx $common $link "$src/cxx_main.cpp" -L. -lcxx_lib $origin -o cxx

# dlopen: a plugin found through $ORIGIN at run time.
$cc $common $link -shared "$src/dlopen_plugin.c" -o libdlopen_plugin.so
$cc $common $link "$src/dlopen_main.c" -ldl $origin -o dlopen

# ifunc: a resolver chooses the implementation at load time.
$cc $common $link "$src/ifunc.c" -o ifunc

sha256sum $(ls | sort) > SHA256SUMS
