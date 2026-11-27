#!/bin/bash
# Builds the guest's runtime inside the image from guest/glibc/Dockerfile, without
# network: glibc, then cs2metal's shim libraries. /w/debian holds Debian's glibc
# 2.31-13+deb11u14 source, already unpacked with Debian's patches; /spec is the
# repository's guest/ directory, read-only. The libraries the guest loads go to
# /w/out, named as the Steam Linux Runtime names them, the shims to /w/out/shims,
# all with their checksums.
set -euo pipefail

rm -rf /w/patched /w/build /w/out
cp -a /w/debian/glibc-2.31 /w/patched
python3 /spec/glibc/patch_glibc.py /w/patched

mkdir /w/build /w/out
cd /w/build
/w/patched/configure --prefix=/usr --disable-werror --enable-kernel=3.2 \
    --enable-stack-protector=strong --disable-profile > configure.log 2>&1
start=$(date +%s)
make -j"$(nproc)" > make.log 2>&1 || { tail -20 make.log; exit 1; }
echo "glibc: built in $(( $(date +%s) - start )) s"

cp elf/ld.so /w/out/ld-linux-x86-64.so.2
cp libc.so /w/out/libc.so.6
cp nptl/libpthread.so /w/out/libpthread.so.0
cp dlfcn/libdl.so /w/out/libdl.so.2
cp math/libm.so /w/out/libm.so.6
cp rt/librt.so /w/out/librt.so.1
# The shims: small Linux libraries cs2metal serves in place of host-bound ones.
# /spec/generated holds what tools/guest-glibc.sh generated for them on the Mac.
mkdir /w/out/shims
# A shim's file name is its soname: the source's name, with ".so" added unless it
# has one (steamclient.c gives steamclient.so, libSDL3.so.0.c gives libSDL3.so.0).
# A .map beside the source is its version script.
for src in /spec/shims/*.c; do
    name=$(basename "$src" .c)
    [[ "$name" == *.so* ]] || name="$name.so"
    map="${src%.c}.map"
    versions=()
    [[ -f "$map" ]] && versions=(-Wl,--version-script="$map")
    # A .libs beside the source lists the guest libraries the shim links.
    libs=()
    [[ -f "${src%.c}.libs" ]] && read -r -a libs < "${src%.c}.libs"
    gcc -O2 -fPIC -shared -nostdlib -fno-stack-protector -g0 -fno-ident \
        -ffile-prefix-map=/spec=. -I/spec/generated -Wall -Werror \
        -Wl,--hash-style=gnu -Wl,-z,now -Wl,--build-id=sha1 -Wl,-soname,"$name" \
        "${versions[@]}" "$src" "${libs[@]}" -o "/w/out/shims/$name"
done
echo "glibc: shims built: $(ls /w/out/shims | tr '\n' ' ')"

cd /w/out
for f in *.so*; do
    slot=$(objdump -d --no-show-raw-insn "$f" | grep -cE 'call.*\*%fs:0x20' || true)
    left=$(objdump -d --no-show-raw-insn "$f" | awk '/^[0-9a-f]+ </{fn=$2} /\tsyscall *$/{print fn}' | sort -u | tr '\n' ' ')
    echo "glibc: $f: $slot calls through the slot; raw syscall in: ${left:-none}"
done
find . -type f ! -name SHA256SUMS | sort | xargs sha256sum > SHA256SUMS
