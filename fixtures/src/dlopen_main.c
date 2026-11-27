// dlopen, dlsym, dlerror, and dlclose, with the plugin found through $ORIGIN.
#include <dlfcn.h>
#include <stdio.h>
#include <string.h>

int main(void) {
    void *missing = dlopen("libcs2metal_absent.so", RTLD_NOW);
    printf("absent library: %s\n", missing ? "loaded?!" : "refused");
    dlerror();

    void *plugin = dlopen("libdlopen_plugin.so", RTLD_NOW | RTLD_LOCAL);
    if (!plugin) {
        printf("dlopen failed: %s\n", dlerror());
        return 1;
    }
    int (*answer)(int) = (int (*)(int))dlsym(plugin, "plugin_answer");
    const char *(*name)(void) = (const char *(*)(void))dlsym(plugin, "plugin_name");
    void *absent = dlsym(plugin, "plugin_absent");
    printf("plugin: %s, answer(20) = %d\n", name(), answer(20));
    printf("absent symbol: %s\n", absent ? "found?!" : "not found");
    printf("dlclose: %d\n", dlclose(plugin));
    return 0;
}
