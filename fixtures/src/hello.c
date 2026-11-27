// The smallest guest: libc output, arguments, environment, and an exit status.
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

int main(int argc, char **argv) {
    printf("hello from a linux guest\n");
    printf("argc=%d\n", argc);
    for (int i = 1; i < argc; i++) printf("argv[%d]=%s\n", i, argv[i]);
    const char *v = getenv("CS2METAL_FIXTURE");
    printf("env=%s\n", v ? v : "(unset)");
    char buffer[32];
    snprintf(buffer, sizeof buffer, "%d-%s", 42, "ok");
    printf("snprintf=%s strlen=%zu\n", buffer, strlen(buffer));
    return 3;
}
