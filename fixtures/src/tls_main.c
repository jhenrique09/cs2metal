// Thread-local storage in the executable (local-exec, through fs) and in a shared
// object (global-dynamic), across threads. Every value is printed after the threads
// join, so the output does not depend on scheduling.
#include <pthread.h>
#include <stdio.h>

int lib_bump(int by);

static __thread int exe_counter = 7;
static __thread char exe_tag[16] = "initial";

#define THREADS 4
static int results[THREADS][3];

static void *worker(void *arg) {
    int id = (int)(long)arg;
    for (int i = 0; i <= id; i++) exe_counter += 10;
    exe_tag[0] = (char)('A' + id);
    results[id][0] = exe_counter;
    results[id][1] = lib_bump(id + 1);
    results[id][2] = exe_tag[0];
    return NULL;
}

int main(void) {
    pthread_t threads[THREADS];
    for (long i = 0; i < THREADS; i++) pthread_create(&threads[i], NULL, worker, (void *)i);
    for (int i = 0; i < THREADS; i++) pthread_join(threads[i], NULL);
    for (int i = 0; i < THREADS; i++)
        printf("thread %d: exe=%d lib=%d tag=%c\n", i, results[i][0], results[i][1], results[i][2]);
    printf("main: exe=%d lib=%d tag=%s\n", exe_counter, lib_bump(0), exe_tag);
    return 0;
}
