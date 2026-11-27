"""Sends every system call of glibc 2.31's x86-64 Linux layer through one slot.

Each `syscall` instruction becomes `lea -128(%rsp), %rsp; call *%fs:0x20;
lea 128(%rsp), %rsp`. The `lea` keeps the red zone intact, and 0x20 is
tcbhead_t.sysinfo, unused on x86-64; i386 calls the kernel through the same field.
T-001 later rewrites the `fs` prefix to `gs`, and the cs2metal host keeps the slot
pointing at its system call entry, which preserves every register a `syscall`
preserves. See docs/LIBC_ROUTE_DECISION.md.

Usage: patch_glibc.py <glibc source tree>. Prints what it changed, and fails if a
site it expects is missing.
"""
import pathlib
import re
import sys

C_CALL = r'"lea -128(%%rsp), %%rsp\n\tcall *%%fs:0x20\n\tlea 128(%%rsp), %%rsp\n\t"'
C_CALL_LAST = r'"lea -128(%%rsp), %%rsp\n\tcall *%%fs:0x20\n\tlea 128(%%rsp), %%rsp"'
ASM_CALL = "lea -128(%rsp), %rsp; call *%fs:0x20; lea 128(%rsp), %rsp"

root = pathlib.Path(sys.argv[1])
linux = root / "sysdeps/unix/sysv/linux/x86_64"

# The C templates and the DO_CALL assembler macro.
sysdep = linux / "sysdep.h"
text = sysdep.read_text()
n_c = text.count('"syscall\\n\\t"')
text = text.replace('"syscall\\n\\t"', C_CALL)
text, n_asm = re.subn(r"^(\s*)syscall;", lambda m: m.group(1) + ASM_CALL + ";", text, flags=re.M)
sysdep.write_text(text)

# Hand-written assembly.
files = sites = 0
for f in sorted(linux.rglob("*.S")):
    t = f.read_text()
    t2, n = re.subn(r"^(\s*)syscall\b", lambda m: m.group(1) + ASM_CALL, t, flags=re.M)
    if n:
        f.write_text(t2)
        files += 1
        sites += n

# TLS_INIT_TP: the arch_prctl that sets the thread pointer, in ld.so.
tls = root / "sysdeps/x86_64/nptl/tls.h"
t = tls.read_text()
n_tls = t.count('asm volatile ("syscall"')
tls.write_text(t.replace('asm volatile ("syscall"', "asm volatile (" + C_CALL_LAST))

print(f"sysdep.h: {n_c} C templates, {n_asm} DO_CALL; {sites} sites in {files} .S files; "
      f"tls.h: {n_tls} TLS_INIT_TP; {n_c + n_asm + sites + n_tls} sites in all")
if (n_c, n_asm, n_tls) != (7, 1, 1) or files < 9:
    sys.exit("unexpected source: the patch did not find every site it expects")
