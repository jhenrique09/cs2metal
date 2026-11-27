# Route C measurement for the cs2metal libc bake-off: replace every `syscall`
# instruction in glibc 2.31's x86-64 Linux syscall layer with a call through the
# unused tcbhead_t.sysinfo slot (fs:0x20), keeping the red zone intact.
import pathlib, re, sys
root = pathlib.Path(sys.argv[1])
C_CALL = r'"lea -128(%%rsp), %%rsp\n\tcall *%%fs:0x20\n\tlea 128(%%rsp), %%rsp\n\t"'
ASM_CALL = 'lea -128(%rsp), %rsp; call *%fs:0x20; lea 128(%rsp), %rsp'
changed = 0
sysdep = root / 'sysdeps/unix/sysv/linux/x86_64/sysdep.h'
text = sysdep.read_text()
n_c = text.count('"syscall\\n\\t"')
text = text.replace('"syscall\\n\\t"', C_CALL)
text, n_asm = re.subn(r'^(\s*)syscall;', lambda m: m.group(1) + ASM_CALL + ';', text, flags=re.M)
sysdep.write_text(text)
changed += n_c + n_asm
files = 0
for f in sorted((root / 'sysdeps/unix/sysv/linux/x86_64').rglob('*.S')):
    t = f.read_text()
    t2, n = re.subn(r'^(\s*)syscall\b', lambda m: m.group(1) + ASM_CALL, t, flags=re.M)
    if n:
        f.write_text(t2); changed += n; files += 1
print(f"sysdep.h: {n_c} C macros, {n_asm} asm macros; {files} .S files; {changed} sites")
