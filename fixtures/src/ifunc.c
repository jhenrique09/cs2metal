// An IFUNC: the loader calls the resolver to choose the implementation. glibc's
// own string functions are IFUNCs too. The output is the same whichever
// implementation the resolver picks.
#include <stdio.h>
#include <string.h>

static int sum_plain(const int *v, int n) {
    int s = 0;
    for (int i = 0; i < n; i++) s += v[i];
    return s;
}

static int sum_wide(const int *v, int n) {
    int s0 = 0, s1 = 0, i = 0;
    for (; i + 1 < n; i += 2) {
        s0 += v[i];
        s1 += v[i + 1];
    }
    return s0 + s1 + (i < n ? v[i] : 0);
}

static int resolved;

static int (*resolve_sum(void))(const int *, int) {
    resolved++;
    __builtin_cpu_init();
    return __builtin_cpu_supports("avx2") ? sum_wide : sum_plain;
}

int sum(const int *v, int n) __attribute__((ifunc("resolve_sum")));

int main(void) {
    int v[9] = {1, 2, 3, 4, 5, 6, 7, 8, 9};
    printf("sum = %d\n", sum(v, 9));
    printf("resolver ran: %s\n", resolved > 0 ? "yes" : "no");
    char text[32] = "ifunc";
    strcat(text, " and glibc");
    printf("strlen('%s') = %zu\n", text, strlen(text));
    return 0;
}
