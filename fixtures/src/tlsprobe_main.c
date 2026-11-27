// Prints what libtlsprobe answers on Linux, from three threads and the main
// thread: the ground truth for the loader's TLS and bridge tests, which make the
// same calls.
#include <pthread.h>
#include <stdio.h>

int probe_bump(int by);
int probe_zero_sum(void);
int probe_tp_is_self(void);
int probe_guarded(int n);
int probe_call_host(int x);

// Exported (-rdynamic), so the library's reference binds here.
int host_echo(int x) { return x * 3; }

#define THREADS 3
static int results[THREADS][3];

static void *worker(void *arg) {
    int id = (int)(long)arg;
    results[id][0] = probe_bump(id + 1);
    results[id][1] = probe_bump(id + 1);
    results[id][2] = probe_zero_sum();
    return NULL;
}

int main(void) {
    pthread_t threads[THREADS];
    for (long i = 0; i < THREADS; i++) pthread_create(&threads[i], NULL, worker, (void *)i);
    for (int i = 0; i < THREADS; i++) pthread_join(threads[i], NULL);
    for (int i = 0; i < THREADS; i++)
        printf("thread %d: bump=%d bump=%d zero_sum=%d\n", i, results[i][0], results[i][1], results[i][2]);
    // One call per statement: argument evaluation order is unspecified in C.
    int bump = probe_bump(10);
    int zero_sum = probe_zero_sum();
    printf("main: bump=%d zero_sum=%d\n", bump, zero_sum);
    int tp_is_self = probe_tp_is_self();
    int guarded = probe_guarded(3);
    int call_host = probe_call_host(5);
    printf("tp_is_self=%d guarded=%d call_host=%d\n", tp_is_self, guarded, call_host);
    return 0;
}
