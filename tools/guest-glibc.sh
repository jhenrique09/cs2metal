#!/usr/bin/env bash
# Builds the guest's C library: Debian's glibc 2.31-13+deb11u14, the Steam Linux
# Runtime's, with every system call sent through one slot (guest/glibc), and
# copies it to target/guest-glibc/, checked against guest/glibc/SHA256SUMS.
#
#   tools/guest-glibc.sh            build and verify
#   tools/guest-glibc.sh --update   rewrite guest/glibc/SHA256SUMS
#
# Runs on the x86-64 Linux host named by CS2METAL_BUILD_HOST, with Docker. Only
# fetching the source uses the network; apt checks its signature, and this script
# checks the tarballs against the checksums below. The host is never written to
# the repository.
set -euo pipefail

root="$(git -C "$(dirname "$0")" rev-parse --show-toplevel)"
host="${CS2METAL_BUILD_HOST:?set CS2METAL_BUILD_HOST to an x86-64 Linux ssh destination}"
update="${1:-}"
work="cs2metal-glibc"
dest="$root/target/guest-glibc"
orig="3dc7704b6166839c37d7047626fd199f3d4c09aca0d90e48c51c31c967dce34e  glibc_2.31.orig.tar.xz"
debian="b173a5551de5c569738ee545beda2988769ed3f83c2b67def5cf862408bebe95  glibc_2.31-13+deb11u14.debian.tar.xz"

echo "guest-glibc: building on the x86-64 host named by CS2METAL_BUILD_HOST"
ssh -o BatchMode=yes "$host" "mkdir -p $work/spec $work/debian"
rsync -a --delete "$root/guest/glibc/" "$host:$work/spec/"
ssh -o BatchMode=yes "$host" bash -s <<REMOTE
set -euo pipefail
cd \$HOME/$work
docker build -q -t cs2metal-glibc-build spec > /dev/null
if [ ! -d debian/glibc-2.31 ]; then
    docker run --rm -u 0 -v \$HOME/$work/debian:/src cs2metal-glibc-build bash -c \
        "apt-get update -qq && cd /src && apt-get source -qq glibc=2.31-13+deb11u14 && chown -R \$(id -u):\$(id -g) /src"
fi
cd debian && printf '%s\n%s\n' "$orig" "$debian" | sha256sum -c --quiet && cd ..
docker run --rm --network none -u \$(id -u):\$(id -g) \
    -v \$HOME/$work:/w -v \$HOME/$work/spec:/spec:ro cs2metal-glibc-build /spec/build.sh
REMOTE
rm -rf "$dest" && mkdir -p "$dest"
rsync -a "$host:$work/out/" "$dest/"

if [ "$update" = "--update" ]; then
    cp "$dest/SHA256SUMS" "$root/guest/glibc/SHA256SUMS"
    echo "guest-glibc: SHA256SUMS updated"
elif diff -u "$root/guest/glibc/SHA256SUMS" "$dest/SHA256SUMS"; then
    echo "guest-glibc: built and byte-identical to guest/glibc/SHA256SUMS"
else
    echo "guest-glibc: the build differs from guest/glibc/SHA256SUMS" >&2
    exit 4
fi
