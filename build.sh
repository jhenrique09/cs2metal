#!/usr/bin/env bash
# Builds everything on this Mac and installs it under ~/cs2-metal/bin.
#
#   ./build.sh                build what is missing or changed, then install
#   ./build.sh --host-only    only the host program; keep the installed guest library
#   ./build.sh --force-guest  build the guest's C library from source again
#
# Three things are built:
#
# 1. The host program, cs2metal: an x86-64 macOS binary that runs under Rosetta 2.
#    Built with cargo.
#
# 2. The guest's C library: Debian's glibc 2.31 with cs2metal's patch
#    (guest/glibc/patch_glibc.py). Its six libraries have tracked checksums,
#    guest/glibc/SHA256SUMS, and nothing is installed that does not match them.
#    They are taken from the first of these that has them:
#
#    - target/guest-glibc, where an earlier build left them;
#    - CS2METAL_GLIBC_FROM=<directory>, six libraries built somewhere else;
#    - a build from source (tools/guest-glibc.sh). glibc builds only with GCC and
#      GNU binutils for Linux, so this is the one step that needs a Linux
#      container: Docker on this Mac (Docker Desktop or OrbStack), or the x86-64
#      Linux host named by CS2METAL_BUILD_HOST. It takes about twelve minutes on
#      an M1 Max, once.
#
# 3. The shim libraries (guest/shims), with Apple's clang and the linker in the
#    Rust toolchain (tools/guest-shims.sh): a few seconds, no container. They are
#    generated for the functions the game's own SDL and this Mac's Steam client
#    export, so the game must be downloaded first (./install.sh does both in the
#    right order).
#
# Needs: the Xcode command line tools, rustup, the game in
# ~/cs2-metal/data/cs2-linux, and Steam for macOS.
set -euo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
. "$here/tools/common.sh"
cd "$here"

host_only=0
force_guest=0
for arg in "$@"; do
    case "$arg" in
        --host-only) host_only=1 ;;
        --force-guest) force_guest=1 ;;
        *) fail "unknown option $arg (see the top of build.sh)" ;;
    esac
done

command -v cargo > /dev/null || fail "cargo not found: install rustup from https://rustup.rs"
target=x86_64-apple-darwin
rustup target list --installed 2> /dev/null | grep -qx "$target" || {
    say "adding the Rust target $target"
    rustup target add "$target"
}

say "building the host program"
cargo build --release --target "$target" -p cs2metal --locked

built="target/guest-glibc"
libraries="ld-linux-x86-64.so.2 libc.so.6 libdl.so.2 libm.so.6 libpthread.so.0 librt.so.1"

# Whether the six libraries in a directory are the ones the repository names.
matches() { (cd "$1" 2> /dev/null && shasum -a 256 -c --quiet "$here/guest/glibc/SHA256SUMS" > /dev/null 2>&1); }

guest_library() {
    if [ "$force_guest" = 0 ] && matches "$built"; then
        say "the guest's C library is built and matches its checksums"
    elif [ "$force_guest" = 0 ] && [ -n "${CS2METAL_GLIBC_FROM:-}" ]; then
        matches "$CS2METAL_GLIBC_FROM" ||
            fail "the libraries in $CS2METAL_GLIBC_FROM do not match guest/glibc/SHA256SUMS"
        say "taking the guest's C library from $CS2METAL_GLIBC_FROM"
        rm -rf "$built" && mkdir -p "$built"
        for library in $libraries; do cp "$CS2METAL_GLIBC_FROM/$library" "$built/"; done
    elif [ -n "${CS2METAL_BUILD_HOST:-}" ] || docker info > /dev/null 2>&1; then
        say "building the guest's C library from source (about twelve minutes in Docker)"
        tools/guest-glibc.sh
    else
        cat >&2 << TEXT
cs2metal: the guest's C library is not built, and this Mac cannot build it as it
cs2metal: is: glibc needs a Linux compiler. Do one of these, then run ./build.sh again:
cs2metal:   - start Docker (OrbStack or Docker Desktop);
cs2metal:   - set CS2METAL_BUILD_HOST=<ssh destination> to an x86-64 Linux host with Docker;
cs2metal:   - set CS2METAL_GLIBC_FROM=<directory> to the six libraries built on another
cs2metal:     machine (its target/guest-glibc); they are checked against the checksums.
TEXT
        exit 1
    fi
}

if [ "$host_only" = 1 ]; then
    [ -f "$guest_dir/libc.so.6" ] || fail "no guest library installed yet: run ./build.sh without --host-only"
else
    [ -f "$game_dir/game/bin/linuxsteamrt64/libSDL3.so.0" ] ||
        fail "the game is not in $game_dir: run ./install.sh, which downloads it and then builds"
    [ -f "$steam_client/steamclient.dylib" ] ||
        fail "Steam for macOS is not installed: the shims are generated from its client library"
    guest_library
    say "building the shims"
    tools/guest-shims.sh "$built"
    # What the SDL shim was built for; run.sh compares it after a game update.
    sdl_exports > "$built/SDL_EXPORTS"
fi

say "installing to $bin_dir"
mkdir -p "$bin_dir" "$data_dir" "$logs_dir"
# Written beside and renamed, so a running game keeps the program it started.
cp "target/$target/release/cs2metal" "$bin_dir/cs2metal.new"
mv "$bin_dir/cs2metal.new" "$host_bin"
if [ "$host_only" = 0 ]; then
    rm -rf "$guest_dir.new"
    cp -R "$built" "$guest_dir.new"
    rm -rf "$guest_dir"
    mv "$guest_dir.new" "$guest_dir"
fi
for script in run.sh run-offline.sh create-shortcut.sh tools/common.sh tools/low-preset.sh \
    tools/steam_shortcuts.py; do
    install -m 755 "$script" "$bin_dir/$(basename "$script")"
done
git rev-parse --short HEAD > "$bin_dir/VERSION" 2> /dev/null || true

say "built and installed; start the game with $bin_dir/run.sh"
