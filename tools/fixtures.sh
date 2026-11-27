#!/usr/bin/env bash
# Builds the Linux fixtures in the pinned Steam Runtime 3 SDK image, runs them
# natively to check their expected output, and copies them to target/fixtures/.
#
#   tools/fixtures.sh            build, run, and verify checksums and outputs
#   tools/fixtures.sh --update   rewrite fixtures/SHA256SUMS and fixtures/expected/
#
# CS2METAL_BUILD_HOST names an x86-64 Linux machine with Docker, as an ssh
# destination (an alias from ~/.ssh/config is best); the build then runs there,
# natively. Without it the build uses the local Docker, emulating linux/amd64.
# The destination is never written to the repository.
set -euo pipefail

root="$(git -C "$(dirname "$0")" rev-parse --show-toplevel)"
image="registry.gitlab.steamos.cloud/steamrt/sniper/sdk@sha256:bb30053d3cd19d35c76ae69b2790dbaa1430448006f8240cb6d7e723f2d1940e"
update="${1:-}"
dest="$root/target/fixtures"
host="${CS2METAL_BUILD_HOST:-}"
# One container run: $1 is the mount mode of the fixture tree (":ro" or ""), the
# rest is the command. Builds and runs have no network.
if [ -n "$host" ]; then
    work="cs2metal-fixtures"
    in_image() {
        ssh -o BatchMode=yes "$host" "docker run --rm --network none -u \$(id -u):\$(id -g) \
            -v \$HOME/$work/src:/src$1 -v \$HOME/$work/out:/out $image ${*:2}"
    }
    ssh -o BatchMode=yes "$host" "rm -rf $work/out && mkdir -p $work/out $work/src"
    rsync -a --delete "$root/fixtures/" "$host:$work/src/"
else
    in_image() {
        docker run --rm --network none -u "$(id -u):$(id -g)" --platform linux/amd64 \
            -v "$root/fixtures:/src$1" -v "$dest:/out" "$image" "${@:2}"
    }
    rm -rf "$dest" && mkdir -p "$dest"
fi

in_image :ro /src/build.sh
if [ "$update" = "--update" ]; then
    in_image "" /src/run.sh --update
    [ -n "$host" ] && rsync -a "$host:$work/src/expected/" "$root/fixtures/expected/"
else
    in_image :ro /src/run.sh
fi
if [ -n "$host" ]; then
    rm -rf "$dest" && mkdir -p "$dest"
    rsync -a "$host:$work/out/" "$dest/"
fi

if [ "$update" = "--update" ]; then
    cp "$dest/SHA256SUMS" "$root/fixtures/SHA256SUMS"
    echo "fixtures: SHA256SUMS and expected outputs updated"
elif diff -u "$root/fixtures/SHA256SUMS" "$dest/SHA256SUMS"; then
    echo "fixtures: built, run, and byte-identical to fixtures/SHA256SUMS"
else
    echo "fixtures: the build differs from fixtures/SHA256SUMS" >&2
    exit 4
fi
