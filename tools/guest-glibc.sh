#!/usr/bin/env bash
# Builds the guest's C library from source: Debian's glibc 2.31-13+deb11u14, the
# Steam Linux Runtime's, with every system call sent through one slot
# (guest/glibc). The six libraries go to target/guest-glibc/ and are checked
# against guest/glibc/SHA256SUMS. The shims are built apart, on the Mac itself
# (tools/guest-shims.sh).
#
#   tools/guest-glibc.sh            build and verify
#   tools/guest-glibc.sh --update   rewrite guest/glibc/SHA256SUMS
#
# glibc builds only with GCC and GNU binutils for Linux, and the checksums hold
# only for one compiler, so the build runs in the pinned Steam Runtime 3 SDK
# image, an x86-64 Linux container:
#
# - on this Mac, with Docker (Docker Desktop or OrbStack), which runs the image
#   under emulation. Everything stays in a Docker volume, cs2metal-glibc-work, on
#   a Linux file system; only the libraries are copied out.
# - on the x86-64 Linux host named by CS2METAL_BUILD_HOST, an ssh destination with
#   Docker, if that variable is set. The host is never written to the repository.
#
# Only fetching the source uses the network; apt checks its signature, and this
# script checks the tarballs against the checksums below.
set -euo pipefail

root="$(git -C "$(dirname "$0")" rev-parse --show-toplevel)"
update="${1:-}"
dest="$root/target/guest-glibc"
image="cs2metal-glibc-build"
source_version="2.31-13+deb11u14"
orig="3dc7704b6166839c37d7047626fd199f3d4c09aca0d90e48c51c31c967dce34e  glibc_2.31.orig.tar.xz"
debian="b173a5551de5c569738ee545beda2988769ed3f83c2b67def5cf862408bebe95  glibc_2.31-13+deb11u14.debian.tar.xz"

build_here() {
    local volume="cs2metal-glibc-work" owner
    owner="$(id -u):$(id -g)"
    docker info > /dev/null 2>&1 || {
        echo "guest-glibc: Docker is not running; start Docker Desktop or OrbStack" >&2
        exit 1
    }
    echo "guest-glibc: building in Docker on this Mac (x86-64 under emulation)"
    run() { docker run --rm --platform linux/amd64 "$@"; }
    docker build -q --platform linux/amd64 -t "$image" "$root/guest/glibc" > /dev/null
    run -u 0 -v "$volume:/w" "$image" bash -c "
        set -euo pipefail
        mkdir -p /w/debian && cd /w/debian
        if [ ! -d glibc-2.31 ]; then
            apt-get update -qq && apt-get source -qq glibc=$source_version
        fi
        printf '%s\n%s\n' '$orig' '$debian' | sha256sum -c --quiet
        chown -R $owner /w"
    run --network none -u "$owner" -v "$volume:/w" -v "$root/guest:/spec:ro" "$image" /spec/glibc/build.sh
    rm -rf "$dest" && mkdir -p "$dest"
    run -u "$owner" -v "$volume:/w:ro" -v "$dest:/dest" "$image" cp -R /w/out/. /dest/
}

build_there() {
    local host="$1" work="cs2metal-glibc"
    echo "guest-glibc: building on the x86-64 host named by CS2METAL_BUILD_HOST"
    ssh -o BatchMode=yes "$host" "mkdir -p $work/spec $work/debian"
    rsync -a --delete "$root/guest/" "$host:$work/spec/"
    ssh -o BatchMode=yes "$host" bash -s << REMOTE
set -euo pipefail
cd \$HOME/$work
docker build -q -t $image spec/glibc > /dev/null
if [ ! -d debian/glibc-2.31 ]; then
    docker run --rm -u 0 -v \$HOME/$work/debian:/src $image bash -c \
        "apt-get update -qq && cd /src && apt-get source -qq glibc=$source_version && chown -R \$(id -u):\$(id -g) /src"
fi
cd debian && printf '%s\n%s\n' "$orig" "$debian" | sha256sum -c --quiet && cd ..
docker run --rm --network none -u \$(id -u):\$(id -g) \
    -v \$HOME/$work:/w -v \$HOME/$work/spec:/spec:ro $image /spec/glibc/build.sh
REMOTE
    rm -rf "$dest" && mkdir -p "$dest"
    rsync -a "$host:$work/out/" "$dest/"
}

if [ -n "${CS2METAL_BUILD_HOST:-}" ]; then
    build_there "$CS2METAL_BUILD_HOST"
else
    build_here
fi

if [ "$update" = "--update" ]; then
    cp "$dest/SHA256SUMS" "$root/guest/glibc/SHA256SUMS"
    echo "guest-glibc: SHA256SUMS updated"
elif diff -u "$root/guest/glibc/SHA256SUMS" "$dest/SHA256SUMS"; then
    echo "guest-glibc: built; the six libraries are byte-identical to guest/glibc/SHA256SUMS"
else
    echo "guest-glibc: the build differs from guest/glibc/SHA256SUMS" >&2
    exit 4
fi
