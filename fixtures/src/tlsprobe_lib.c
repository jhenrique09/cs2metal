// Thread-local storage in every model a shared object can use, the stack
// protector, a direct read of the thread pointer, and a call out to a function
// the program provides. No C library, so the loader can run it before it has one.
// Built with -fstack-protector-all.
__thread int gd_counter = 5;                                         // global-dynamic
static __thread int ld_counter = 7;                                  // local-dynamic
__attribute__((tls_model("initial-exec"))) __thread int ie_counter = 9;  // initial-exec
__thread char zeroed[32];                                            // .tbss

int host_echo(int x);  // provided by whoever loads this library

int probe_bump(int by) {
    gd_counter += by;
    ld_counter += 2 * by;
    ie_counter += 3 * by;
    zeroed[by % 32] += 1;
    return gd_counter * 1000000 + ld_counter * 1000 + ie_counter;
}

int probe_zero_sum(void) {
    int sum = 0;
    for (int i = 0; i < 32; i++) sum += zeroed[i];
    return sum;
}

// glibc's thread control block starts with a pointer to itself.
int probe_tp_is_self(void) {
    void **tp;
    __asm__("mov %%fs:0, %0" : "=r"(tp));
    return *tp == (void *)tp;
}

int probe_guarded(int n) {
    volatile char buffer[64];
    for (int i = 0; i < 64; i++) buffer[i] = (char)(n + i);
    int sum = 0;
    for (int i = 0; i < 64; i++) sum += buffer[i];
    return sum;
}

int probe_call_host(int x) { return host_echo(x) + 1; }
