/*
 * Probes how Rosetta 2 treats the x86-64 segment registers that Linux code uses
 * for thread-local storage (fs) and that macOS uses for its own (gs).
 *
 * Each case runs in its own process because several of them hang or abort the
 * process under Rosetta; run.sh enforces a timeout per case.
 *
 * Build: clang -arch x86_64 -O1 -mfsgsbase -mavx2 probe.c -o probe
 */
#include <i386/user_ldt.h>
#include <architecture/i386/table.h>
#include <pthread.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <time.h>
#include <unistd.h>

/* macOS machdep call thread_fast_set_cthread_self: sets the calling thread's GS base. */
#define SET_GS_BASE 0x3000003

static uint64_t block[4] = {0x1111, 0x2222, 0x3333, 0x4444};

static long set_gs_base(const void *base) {
    long ret;
    __asm__ volatile("syscall" : "=a"(ret) : "a"(SET_GS_BASE), "D"(base) : "rcx", "r11", "memory");
    return ret;
}

static uint64_t gs_word(int index) {
    uint64_t v;
    __asm__ volatile("mov %%gs:(,%1,8), %0" : "=r"(v) : "r"((uint64_t)index));
    return v;
}

/*
 * The base to restore is the TSD array inside the pthread structure, not
 * pthread_self() itself: find the address whose first 12 words equal gs:[0..96].
 */
static uintptr_t host_tsd_base(void) {
    uint64_t want[12];
    for (int i = 0; i < 12; i++) want[i] = gs_word(i);
    uintptr_t self = (uintptr_t)pthread_self();
    for (uintptr_t at = self; at < self + 0x400; at += 8) {
        if (memcmp((const void *)at, want, sizeof want) == 0) return at;
    }
    return 0;
}

static void cpuid(unsigned leaf, unsigned sub, unsigned r[4]) {
    __asm__ volatile("cpuid" : "=a"(r[0]), "=b"(r[1]), "=c"(r[2]), "=d"(r[3]) : "a"(leaf), "c"(sub));
}

static int case_cpuid(void) {
    unsigned r[4];
    char vendor[13];
    cpuid(0, 0, r);
    memcpy(vendor, &r[1], 4);
    memcpy(vendor + 4, &r[3], 4);
    memcpy(vendor + 8, &r[2], 4);
    vendor[12] = 0;
    printf("vendor=%s max_leaf=%u ", vendor, r[0]);
    cpuid(1, 0, r);
    printf("sse4.2=%u avx=%u f16c=%u ", (r[2] >> 20) & 1, (r[2] >> 28) & 1, (r[2] >> 29) & 1);
    cpuid(7, 0, r);
    printf("fsgsbase=%u avx2=%u bmi2=%u ", r[1] & 1, (r[1] >> 5) & 1, (r[1] >> 8) & 1);
    unsigned lo, hi;
    __asm__ volatile("xgetbv" : "=a"(lo), "=d"(hi) : "c"(0));
    printf("xcr0=%#x\n", lo);
    return 0;
}

static int case_avx2(void) {
    __asm__ volatile("vpcmpeqd %%ymm0, %%ymm0, %%ymm0\n\tvpaddd %%ymm0, %%ymm0, %%ymm1" ::: "xmm0", "xmm1");
    printf("AVX2 instructions executed\n");
    return 0;
}

static int case_rdfsbase(void) {
    uint64_t base;
    __asm__ volatile("rdfsbase %0" : "=r"(base));
    printf("rdfsbase=%#llx\n", (unsigned long long)base);
    return 0;
}

static int case_wrfsbase(void) {
    uint64_t v;
    __asm__ volatile("wrfsbase %0" ::"r"((uint64_t)block));
    __asm__ volatile("mov %%fs:8, %0" : "=r"(v));
    printf("fs:8=%#llx (0x2222 expected)\n", (unsigned long long)v);
    return 0;
}

static int case_fs_read(void) {
    uint16_t selector;
    uint64_t v;
    __asm__ volatile("mov %%fs, %0" : "=r"(selector));
    printf("fs selector=%#x; reading fs:0 ... ", selector);
    fflush(stdout);
    __asm__ volatile("mov %%fs:0, %0" : "=r"(v));
    printf("fs:0=%#llx\n", (unsigned long long)v);
    return 0;
}

static int ldt(int start) {
    union ldt_entry e;
    memset(&e, 0, sizeof e);
    uintptr_t base = (uintptr_t)block;
    e.data.base00 = base & 0xffff;
    e.data.base16 = (base >> 16) & 0xff;
    e.data.base24 = (base >> 24) & 0xff;
    e.data.limit00 = 0xffff;
    e.data.limit16 = 0xf;
    e.data.type = DESC_DATA_WRITE;
    e.data.dpl = 3;
    e.data.present = 1;
    e.data.stksz = 1;
    e.data.granular = 1;
    int index = i386_set_ldt(start, &e, 1);
    printf("i386_set_ldt=%d\n", index);
    return index < 0;
}

static int case_gs_swap(void) {
    uintptr_t host = host_tsd_base();
    if (!host) {
        printf("host TSD base not found\n");
        return 1;
    }
    set_gs_base(block);
    uint64_t seen = gs_word(1);
    set_gs_base((const void *)host);

    pthread_key_t key;
    pthread_key_create(&key, NULL);
    pthread_setspecific(key, block);
    int tsd_ok = pthread_getspecific(key) == block;

    enum { PAIRS = 1000000 };
    uint64_t t0 = clock_gettime_nsec_np(CLOCK_UPTIME_RAW);
    for (int i = 0; i < PAIRS; i++) {
        set_gs_base(block);
        set_gs_base((const void *)host);
    }
    uint64_t t1 = clock_gettime_nsec_np(CLOCK_UPTIME_RAW);
    printf("gs:8=%#llx (0x2222 expected) host_tsd=self+%#lx pthread_tsd_after_restore=%s "
           "swap_pair=%.0f ns\n",
           (unsigned long long)seen, (unsigned long)(host - (uintptr_t)pthread_self()),
           tsd_ok ? "ok" : "BROKEN", (double)(t1 - t0) / PAIRS);
    return !(seen == 0x2222 && tsd_ok);
}

int main(int argc, char **argv) {
    setvbuf(stdout, NULL, _IONBF, 0);
    const char *c = argc > 1 ? argv[1] : "";
    if (!strcmp(c, "cpuid")) return case_cpuid();
    if (!strcmp(c, "avx2")) return case_avx2();
    if (!strcmp(c, "rdfsbase")) return case_rdfsbase();
    if (!strcmp(c, "wrfsbase")) return case_wrfsbase();
    if (!strcmp(c, "fs-read")) return case_fs_read();
    if (!strcmp(c, "ldt-auto")) return ldt(LDT_AUTO_ALLOC);
    if (!strcmp(c, "ldt-fixed")) return ldt(32);
    if (!strcmp(c, "gs-swap")) return case_gs_swap();
    fprintf(stderr, "usage: probe cpuid|avx2|rdfsbase|wrfsbase|fs-read|ldt-auto|ldt-fixed|gs-swap\n");
    return 2;
}
