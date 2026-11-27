// The clocks, and the waits that are timed against them: glibc reads the time
// through the vDSO where there is one, and gives the kernel absolute deadlines
// computed from what it read.
#include <errno.h>
#include <pthread.h>
#include <stdio.h>
#include <sys/time.h>
#include <time.h>

#define SECOND 1000000000LL
#define MS 1000000LL

static long long ns(clockid_t clock) {
    struct timespec t;
    if (clock_gettime(clock, &t) != 0) return -1;
    return t.tv_sec * SECOND + t.tv_nsec;
}

static void say(const char *what, int ok) { printf("%s: %s\n", what, ok ? "ok" : "FAILED"); }

static struct timespec after(clockid_t clock, long long delay) {
    long long at = ns(clock) + delay;
    struct timespec t = {at / SECOND, at % SECOND};
    return t;
}

// A wait on a condition nobody signals, with a deadline 50 ms ahead on `clock`.
static void timed_wait(clockid_t clock, const char *what) {
    pthread_condattr_t attr;
    pthread_cond_t cond;
    pthread_mutex_t mutex = PTHREAD_MUTEX_INITIALIZER;
    pthread_condattr_init(&attr);
    pthread_condattr_setclock(&attr, clock);
    pthread_cond_init(&cond, &attr);
    struct timespec until = after(clock, 50 * MS);
    long long before = ns(CLOCK_MONOTONIC);
    int status;
    pthread_mutex_lock(&mutex);
    do status = pthread_cond_timedwait(&cond, &mutex, &until);
    while (status == 0);
    pthread_mutex_unlock(&mutex);
    long long waited = ns(CLOCK_MONOTONIC) - before;
    say(what, status == ETIMEDOUT && waited >= 45 * MS && waited < SECOND);
}

int main(void) {
    long long last = ns(CLOCK_MONOTONIC);
    int ordered = last > 0;
    for (int i = 0; i < 1000; i++) {
        long long now = ns(CLOCK_MONOTONIC);
        if (now < last) ordered = 0;
        last = now;
    }
    say("the monotonic clock never goes back", ordered);

    struct timeval tv;
    gettimeofday(&tv, NULL);
    long long wall = ns(CLOCK_REALTIME);
    time_t seconds = time(NULL);
    long long day = tv.tv_sec * SECOND + tv.tv_usec * 1000LL;
    say("gettimeofday agrees with the realtime clock", wall >= day && wall - day < SECOND);
    say("time agrees with the realtime clock",
        seconds >= wall / SECOND && seconds - wall / SECOND <= 1);
    say("the realtime clock is after 2025", wall / SECOND > 1735689600LL);

    long long before = ns(CLOCK_MONOTONIC);
    struct timespec nap = {0, 30 * MS};
    nanosleep(&nap, NULL);
    long long slept = ns(CLOCK_MONOTONIC) - before;
    say("a sleep lasts as long by the monotonic clock", slept >= 30 * MS && slept < SECOND);

    before = ns(CLOCK_MONOTONIC);
    struct timespec until = after(CLOCK_MONOTONIC, 30 * MS);
    clock_nanosleep(CLOCK_MONOTONIC, TIMER_ABSTIME, &until, NULL);
    slept = ns(CLOCK_MONOTONIC) - before;
    say("a sleep until a monotonic deadline ends on it", slept >= 29 * MS && slept < SECOND);

    timed_wait(CLOCK_MONOTONIC, "a wait with a monotonic deadline ends on it");
    timed_wait(CLOCK_REALTIME, "a wait with a realtime deadline ends on it");

    say("the thread's processor-time clock answers", ns(CLOCK_THREAD_CPUTIME_ID) >= 0);
    struct timespec t;
    errno = 0;
    say("an unknown clock is refused", clock_gettime(12345, &t) == -1 && errno == EINVAL);
    return 0;
}
