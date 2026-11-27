#!/usr/bin/env bash
# Route C of the libc bake-off: builds glibc 2.31 from its official source with
# every `syscall` instruction of the x86-64 Linux layer replaced by a call through
# one slot, then counts the raw `syscall` instructions left in the binaries.
# Runs on the x86-64 Linux host named by CS2METAL_BUILD_HOST, which needs Docker
# and network access to ftp.gnu.org. Nothing it builds comes back to the Mac.
set -euo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
root="$(git -C "$here" rev-parse --show-toplevel)"
host="${CS2METAL_BUILD_HOST:?set CS2METAL_BUILD_HOST to an x86-64 Linux ssh destination}"
work="cs2metal-glibc"
sha="9246fe44f68feeec8c666bb87973d590ce0137cca145df014c72ec95be9ffd17"

ssh -o BatchMode=yes "$host" "mkdir -p $work"
rsync -a "$root/guest/glibc/patch_glibc.py" "$root/guest/glibc/Dockerfile" "$host:$work/spike/"
ssh -o BatchMode=yes "$host" bash -s <<REMOTE
set -euo pipefail
cd $work
[ -f glibc-2.31.tar.xz ] || curl -fsSL https://ftp.gnu.org/gnu/glibc/glibc-2.31.tar.xz -o glibc-2.31.tar.xz
echo "$sha  glibc-2.31.tar.xz" | sha256sum -c --quiet
rm -rf glibc-2.31 build && tar -xJf glibc-2.31.tar.xz && mkdir build
python3 spike/patch_glibc.py glibc-2.31
docker build -q -t cs2metal-glibc-build spike >/dev/null
docker run --rm --network none -u \$(id -u):\$(id -g) -v \$HOME/$work:/w cs2metal-glibc-build bash -c '
    cd /w/build
    ../glibc-2.31/configure --prefix=/usr --disable-werror --enable-kernel=3.2 --disable-profile > configure.log 2>&1
    start=\$(date +%s); make -j\$(nproc) > make.log 2>&1; echo "build: status \$?, \$(( \$(date +%s) - start )) s"
    for f in libc.so nptl/libpthread.so rt/librt.so elf/ld.so; do
        raw=\$(objdump -d --no-show-raw-insn \$f | grep -cE "\\ssyscall\\s*\$" || true)
        slot=\$(objdump -d --no-show-raw-insn \$f | grep -cE "call.*\\*%fs:0x20" || true)
        left=\$(objdump -d --no-show-raw-insn \$f | awk "/^[0-9a-f]+ </{fn=\\\$2} /\\tsyscall *\$/{print fn}" | sort -u | tr "\\n" " ")
        echo "\$f: \$slot through the slot, \$raw raw: \$left"
    done'
REMOTE
