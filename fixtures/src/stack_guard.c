// The stack protector reads its canary from fs:0x28 in every protected function.
// Built with -fstack-protector-all; it must run without tripping the guard.
#include <stdio.h>
#include <string.h>

static int fill(char *out, size_t size, int seed) {
    char local[64];
    for (size_t i = 0; i < sizeof local; i++) local[i] = (char)('a' + (seed + i) % 26);
    memcpy(out, local, size < sizeof local ? size : sizeof local);
    return local[seed % 64];
}

static int recurse(int depth) {
    char frame[128];
    memset(frame, depth, sizeof frame);
    return depth == 0 ? frame[0] : frame[1] + recurse(depth - 1);
}

int main(void) {
    char buffer[16];
    int c = fill(buffer, sizeof buffer, 5);
    printf("fill: %.16s %c\n", buffer, c);
    printf("recurse: %d\n", recurse(20));
    printf("stack guard: intact\n");
    return 0;
}
