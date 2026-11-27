#!/usr/bin/env bash
# Builds the shim libraries (guest/shims) on this Mac, without a container: small
# x86-64 Linux libraries that cs2metal serves in place of the game's SDL, the
# Vulkan loader, and the Steam client library. Each exports the functions of the
# library it stands for and forwards them to the host through a table the host
# fills.
#
#   tools/guest-shims.sh [guest-library-dir]
#
# The shims go to <guest-library-dir>/shims (default target/guest-glibc), beside
# the guest's C library, which the SDL shim links for its one thread.
#
# Apple's clang compiles for x86-64 Linux as it is, and the shims use no C library
# headers. The linker is the LLD that every Rust toolchain ships, rust-lld. The
# two export lists come from the owner's own files: the game's SDL and this Mac's
# Steam client library (tools/sdl-shim-names.sh, tools/steam-shim-names.sh).
set -euo pipefail

root="$(git -C "$(dirname "$0")" rev-parse --show-toplevel)"
guest="${1:-$root/target/guest-glibc}"
generated="$root/target/guest-shims"
out="$guest/shims"

[ -f "$guest/libpthread.so.0" ] || {
    echo "guest-shims: no guest C library in $guest" >&2
    exit 1
}
sysroot="$(rustc --print sysroot)"
host="$(rustc -vV | sed -n 's/^host: //p')"
lld="$sysroot/lib/rustlib/$host/bin/rust-lld"
[ -x "$lld" ] || {
    echo "guest-shims: no rust-lld in the Rust toolchain at $lld" >&2
    exit 1
}

"$root/tools/steam-shim-names.sh" > /dev/null
"$root/tools/sdl-shim-names.sh" > /dev/null

work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
rm -rf "$out" && mkdir -p "$out"
built=()
for src in "$root"/guest/shims/*.c; do
    # A shim's file name is its soname: the source's name, with ".so" added unless
    # it has one (steamclient.c gives steamclient.so, libSDL3.so.0.c libSDL3.so.0).
    name="$(basename "$src" .c)"
    case "$name" in *.so*) ;; *) name="$name.so" ;; esac
    clang -target x86_64-unknown-linux-gnu -O2 -fPIC -ffreestanding -fno-stack-protector \
        -g0 -fno-ident -ffile-prefix-map="$root"=. -I"$generated" -Wall -Werror \
        -c "$src" -o "$work/$name.o"
    link=(-flavor gnu -shared --hash-style=gnu -z now --build-id=sha1 -soname "$name")
    # A .map beside the source is its version script.
    map="${src%.c}.map"
    [ -f "$map" ] && link+=(--version-script="$map")
    link+=("$work/$name.o")
    # A .libs beside the source lists the guest libraries the shim links.
    if [ -f "${src%.c}.libs" ]; then
        for lib in $(cat "${src%.c}.libs"); do
            case "$lib" in
                -lpthread) link+=("$guest/libpthread.so.0") ;;
                *) echo "guest-shims: $src asks for $lib, which this script does not know" >&2; exit 1 ;;
            esac
        done
    fi
    "$lld" "${link[@]}" -o "$out/$name"
    built+=("$name")
done
echo "guest-shims: built ${built[*]}"
