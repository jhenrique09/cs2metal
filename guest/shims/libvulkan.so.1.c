// cs2metal's Linux stand-in for the Vulkan loader (libvulkan.so.1).
//
// cs2metal serves this library wherever the guest opens libvulkan.so.1. Its
// exports are the loader's global entry points; each jumps through a table,
// which the constructor asks the cs2metal host to fill with entries into the
// macOS Vulkan loader across the bridge, MoltenVK beneath it. Every other
// function the game reaches through vkGetInstanceProcAddr and
// vkGetDeviceProcAddr, which the host answers with bridged entries too.
// See docs/VULKAN_BRIDGE.md.
//
// It works only under cs2metal: the constructor's call goes through the guest's
// system call slot, which only the cs2metal host fills.

#define VULKAN_SHIM_NAMES(X)                      \
    X(vkGetInstanceProcAddr, 0)                   \
    X(vkGetDeviceProcAddr, 1)                     \
    X(vkCreateInstance, 2)                        \
    X(vkEnumerateInstanceExtensionProperties, 3)  \
    X(vkEnumerateInstanceLayerProperties, 4)      \
    X(vkEnumerateInstanceVersion, 5)              \
    X(vk_icdGetInstanceProcAddr, 6)

#define AS_STRING(name, i) #name,
static const char *const names[] = {VULKAN_SHIM_NAMES(AS_STRING)};

#define COUNT (sizeof names / sizeof names[0])

/* Filled by the host; read by the trampolines below. */
__attribute__((visibility("hidden"))) unsigned long cs2metal_vulkan_table[COUNT];

#define STUB(name, i)                                 \
    ".globl " #name "\n"                              \
    ".type " #name ",@function\n"                     \
    #name ":\n"                                       \
    "    jmp *cs2metal_vulkan_table+" #i "*8(%rip)\n" \
    ".size " #name ",.-" #name "\n"

__asm__(".text\n" VULKAN_SHIM_NAMES(STUB));

/* cs2metal's private system call: fill `table` for `names`. */
#define CS2METAL_BIND_VULKAN 0x43530003L

__attribute__((constructor)) static void bind(void) {
    long result;
    register unsigned long *table __asm__("rdi") = cs2metal_vulkan_table;
    register const char *const *list __asm__("rsi") = names;
    register unsigned long count __asm__("rdx") = COUNT;
    __asm__ volatile(
        "lea -128(%%rsp), %%rsp\n\t"
        "call *%%fs:0x20\n\t"
        "lea 128(%%rsp), %%rsp"
        : "=a"(result)
        : "0"(CS2METAL_BIND_VULKAN), "r"(table), "r"(list), "r"(count)
        : "rcx", "r11", "memory", "cc");
    (void)result;
}
