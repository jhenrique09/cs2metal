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

/* cs2metal's private system call: fill `table` for `names`. */
#define CS2METAL_BIND_SDL 0x43530002L

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
}
