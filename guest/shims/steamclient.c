// cs2metal's Linux stand-in for the Steam client library (steamclient.so).
//
// The game's own libsteam_api.so loads ~/.steam/sdk64/steamclient.so, as it does
// on Linux; cs2metal serves this library at that path. Each function the macOS
// client library exports jumps through a table, which the constructor asks the
// cs2metal host to fill, by name, with entries into the owner's macOS
// steamclient.dylib across the bridge. Nothing here emulates Steam: every call
// reaches the real client. See docs/STEAM_BRIDGE_DESIGN.md.
//
// It works only under cs2metal: the constructor's call goes through the guest's
// system call slot, which only the cs2metal host fills.

/* STEAM_SHIM_NAMES(X): X(name, index) for every function the shim exports, in
   table order: those the owner's macOS client library exports, then three the
   game's libsteam_api.so also looks up, which the host answers itself. The list
   is generated from the owner's library by tools/steam-shim-names.sh, so that no
   copy of Valve's export list is tracked. */
#include "steamclient_names.h"

#define AS_STRING(name, i) #name,
static const char *const names[] = {STEAM_SHIM_NAMES(AS_STRING)};

#define COUNT (sizeof names / sizeof names[0])

/* Filled by the host; read by the trampolines below. */
__attribute__((visibility("hidden"))) unsigned long cs2metal_steam_table[COUNT];

#define STUB(name, i)                                \
    ".globl " #name "\n"                             \
    ".type " #name ",@function\n"                    \
    #name ":\n"                                      \
    "    jmp *cs2metal_steam_table+" #i "*8(%rip)\n" \
    ".size " #name ",.-" #name "\n"

__asm__(".text\n" STEAM_SHIM_NAMES(STUB));

/* cs2metal's private system call: fill `table` for `names`. */
#define CS2METAL_BIND_STEAM 0x43530001L

__attribute__((constructor)) static void bind(void) {
    long result;
    register unsigned long *table __asm__("rdi") = cs2metal_steam_table;
    register const char *const *list __asm__("rsi") = names;
    register unsigned long count __asm__("rdx") = COUNT;
    __asm__ volatile(
        "lea -128(%%rsp), %%rsp\n\t"
        "call *%%fs:0x20\n\t"
        "lea 128(%%rsp), %%rsp"
        : "=a"(result)
        : "0"(CS2METAL_BIND_STEAM), "r"(table), "r"(list), "r"(count)
        : "rcx", "r11", "memory", "cc");
    (void)result;
}
