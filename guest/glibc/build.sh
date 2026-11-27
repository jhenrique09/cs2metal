#!/bin/bash
# Builds the guest's glibc inside the image from guest/glibc/Dockerfile, without
# network. /w/debian holds Debian's glibc 2.31-13+deb11u14 source, already
# unpacked with Debian's patches; /spec is this directory, read-only. The
# libraries the guest loads go to /w/out, named as the Steam Linux Runtime names
# them, with their checksums.
set -euo pipefail

rm -rf /w/patched /w/build /w/out
cp -a /w/debian/glibc-2.31 /w/patched
python3 /spec/patch_glibc.py /w/patched

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
cd /w/out
for f in *.so*; do
    slot=$(objdump -d --no-show-raw-insn "$f" | grep -cE 'call.*\*%fs:0x20' || true)
    left=$(objdump -d --no-show-raw-insn "$f" | awk '/^[0-9a-f]+ </{fn=$2} /\tsyscall *$/{print fn}' | sort -u | tr '\n' ' ')
    echo "glibc: $f: $slot calls through the slot; raw syscall in: ${left:-none}"
done
sha256sum $(ls | sort) > SHA256SUMS
