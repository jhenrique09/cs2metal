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
# Executables are position independent, as cs2 is: gcc 10 does not make PIEs by
# default, and a fixed-address executable at 0x400000 falls inside the host's
# __PAGEZERO.
pie="-fPIE -pie"
origin="-Wl,-rpath,\$ORIGIN"

mkdir -p "$out"
cd "$out"

# hello: libc only.
$cc $common $pie $link "$src/hello.c" -o hello

# tls: local-exec TLS in the executable, global-dynamic TLS in a library, threads.
$cc $common $link -shared "$src/tls_lib.c" -o libtls_lib.so
$cc $common $pie $link -pthread "$src/tls_main.c" -L. -ltls_lib $origin -o tls

# stack_guard: every function reads the canary at fs:0x28.
$cc $common $pie $link -fstack-protector-all "$src/stack_guard.c" -o stack_guard

# cxx: exceptions across a shared object, iostream, static constructors.
$cxx $common $link -shared "$src/cxx_lib.cpp" -o libcxx_lib.so
$cxx $common $pie $link "$src/cxx_main.cpp" -L. -lcxx_lib $origin -o cxx

# dlopen: a plugin found through $ORIGIN at run time.
$cc $common $link -shared "$src/dlopen_plugin.c" -o libdlopen_plugin.so
$cc $common $pie $link "$src/dlopen_main.c" -ldl $origin -o dlopen

# ifunc: a resolver chooses the implementation at load time.
$cc $common $pie $link "$src/ifunc.c" -o ifunc

# scope: symbol versions, a weak definition, interposition, DT_NEEDED via $ORIGIN.
$cc $common $link -shared "$src/scope_base.c" -Wl,--version-script="$src/scope_base.map" \
    -Wl,-soname,libscope_base.so -o libscope_base.so
$cc $common $link -shared "$src/scope_over.c" -Wl,-soname,libscope_over.so -o libscope_over.so
$cc $common $link -shared "$src/scope_user.c" -L. -Wl,--no-as-needed -lscope_over -lscope_base \
    -Wl,--as-needed $origin -Wl,-soname,libscope_user.so -o libscope_user.so
$cc $common $pie $link "$src/scope_main.c" -L. -lscope_user $origin -o scope

# tlsprobe: every TLS model of a shared object, the stack protector, the thread
# pointer, and a call to a function the program provides; the library needs no
# C library.
$cc $common $link -shared -fstack-protector-all "$src/tlsprobe_lib.c" \
    -Wl,-soname,libtlsprobe.so -o libtlsprobe.so
$cc $common $pie $link -rdynamic -pthread "$src/tlsprobe_main.c" -L. -ltlsprobe $origin -o tlsprobe

# clock: the clocks through the vDSO, and sleeps and waits timed against them.
$cc $common $pie $link -pthread "$src/clock.c" -o clock

sha256sum $(ls | sort) > SHA256SUMS
