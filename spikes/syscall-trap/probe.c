// Route B of the libc bake-off (roadmap step C3) would keep glibc's own code and
// turn each 2-byte `syscall` into a 2-byte trap, emulated by a signal handler,
// because under Rosetta 2 a Linux `syscall` runs as a macOS one. This measures
// what one such round trip costs here, next to a plain call.
//
// Usage: probe int3|ud2. Each case runs ROUNDS traps; run.sh runs each in its own
// process with a timeout.
#include <signal.h>
#include <stdint.h>
#include <stdio.h>
#include <string.h>
#include <sys/ucontext.h>
#include <time.h>

#ifndef ROUNDS
#define ROUNDS 2000
#endif

static void emulate(int sig, siginfo_t *info, void *context) {
    (void)info;
    ucontext_t *uc = context;
    // Stand-in for a translated system call: a result in rax, then resume.
    uc->uc_mcontext->__ss.__rax = 42;
    if (sig == SIGILL) uc->uc_mcontext->__ss.__rip += 2;  // skip ud2; int3 has advanced
}

static uint64_t now(void) { return clock_gettime_nsec_np(CLOCK_UPTIME_RAW); }

__attribute__((noinline)) static long plain_call(long x) {
    __asm__ volatile("" ::: "memory");
    return x + 42;
}

int main(int argc, char **argv) {
    setvbuf(stdout, NULL, _IONBF, 0);
    const char *which = argc > 1 ? argv[1] : "int3";
    struct sigaction sa;
    memset(&sa, 0, sizeof sa);
    sa.sa_sigaction = emulate;
    sa.sa_flags = SA_SIGINFO;
    sigaction(SIGILL, &sa, NULL);
    sigaction(SIGTRAP, &sa, NULL);

    long sink = 0;
    uint64_t t0 = now();
    for (int i = 0; i < ROUNDS; i++) sink += plain_call(i);
    uint64_t t1 = now();
    if (!strcmp(which, "ud2")) {
        for (int i = 0; i < ROUNDS; i++) {
            long r;
            __asm__ volatile("ud2" : "=a"(r) : : "memory");
            sink += r;
        }
    } else {
        for (int i = 0; i < ROUNDS; i++) {
            long r;
            __asm__ volatile("int3" : "=a"(r) : : "memory");
            sink += r;
        }
    }
    uint64_t t2 = now();
    printf("plain call %.1f ns; %s trap round trip %.0f ns (%d rounds, check %ld)\n",
           (double)(t1 - t0) / ROUNDS, which, (double)(t2 - t1) / ROUNDS, ROUNDS, sink % 7);
    return 0;
}
