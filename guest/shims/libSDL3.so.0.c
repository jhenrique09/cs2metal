// cs2metal's Linux stand-in for the game's SDL 3 library (libSDL3.so.0).
//
// cs2metal serves this library where the game loads libSDL3.so.0 from. It
// exports every function the game's own SDL exports, with the same symbol
// version; each jumps through a table, which the constructor asks the cs2metal
// host to fill, by name, with entries into an x86-64 SDL 3 for macOS across the
// bridge. Windows, input, and audio are then macOS's. See docs/SDL_BRIDGE.md.
//
// It works only under cs2metal: the constructor's call goes through the guest's
// system call slot, which only the cs2metal host fills.

/* SDL_SHIM_NAMES(X): X(name, index) for every function the game's SDL exports,
   generated from it by tools/sdl-shim-names.sh. */
#include "libSDL3_names.h"

#define AS_STRING(name, i) #name,
static const char *const names[] = {SDL_SHIM_NAMES(AS_STRING)};

#define COUNT (sizeof names / sizeof names[0])

/* Filled by the host; read by the trampolines below. */
__attribute__((visibility("hidden"))) unsigned long cs2metal_sdl_table[COUNT];

#define STUB(name, i)                              \
    ".globl " #name "\n"                           \
    ".type " #name ",@function\n"                  \
    #name ":\n"                                    \
    "    jmp *cs2metal_sdl_table+" #i "*8(%rip)\n" \
    ".size " #name ",.-" #name "\n"

__asm__(".text\n" SDL_SHIM_NAMES(STUB));

/* glibc's thread functions, declared here so that the shim needs no C library
   headers and builds with any compiler that targets x86-64 Linux. */
typedef unsigned long pthread_t;
int pthread_create(pthread_t *thread, const void *attributes, void *(*start)(void *), void *argument);
int pthread_detach(pthread_t thread);

/* cs2metal's private system calls: fill `table` for `names`; wait for the next
   guest callback to run. */
#define CS2METAL_BIND_SDL 0x43530002L
#define CS2METAL_SDL_NEXT_CALLBACK 0x43530004L

static long host_call(long number, unsigned long a0) {
    long result;
    register unsigned long r0 __asm__("rdi") = a0;
    __asm__ volatile(
        "lea -128(%%rsp), %%rsp\n\t"
        "call *%%fs:0x20\n\t"
        "lea 128(%%rsp), %%rsp"
        : "=a"(result)
        : "0"(number), "r"(r0)
        : "rdx", "rsi", "rcx", "r11", "memory", "cc");
    return result;
}

/* A guest thread for the callbacks host SDL would make on its own threads, such
   as an audio stream's: the host hands it each call to make, which it makes in
   the guest, and asks for the next. */
static void *callbacks(void *unused) {
    (void)unused;
    for (;;) {
        unsigned long call[5];
        if (host_call(CS2METAL_SDL_NEXT_CALLBACK, (unsigned long)call) != 0)
            continue;
        ((void (*)(unsigned long, unsigned long, unsigned long, unsigned long))call[0])(
            call[1], call[2], call[3], call[4]);
    }
    return 0;
}

__attribute__((constructor)) static void bind(void) {
    long result;
    register unsigned long *table __asm__("rdi") = cs2metal_sdl_table;
    register const char *const *list __asm__("rsi") = names;
    register unsigned long count __asm__("rdx") = COUNT;
    __asm__ volatile(
        "lea -128(%%rsp), %%rsp\n\t"
        "call *%%fs:0x20\n\t"
        "lea 128(%%rsp), %%rsp"
        : "=a"(result)
        : "0"(CS2METAL_BIND_SDL), "r"(table), "r"(list), "r"(count)
        : "rcx", "r11", "memory", "cc");
    (void)result;
    pthread_t thread;
    if (pthread_create(&thread, 0, callbacks, 0) == 0)
        pthread_detach(thread);
}
