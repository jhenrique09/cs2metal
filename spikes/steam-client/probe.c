// H-2 probe: will the macOS Steam client serve app 730 to an x86-64 process?
//
// Loads the Steamworks library that ships inside the owner's Steam installation,
// initializes it as app 730 (from the SteamAppId environment variable, the way a
// game started outside Steam declares itself), and asks read-only questions: is a
// user signed in, is it an individual account, which app the client sees, and
// whether the account holds app 730. Then it shuts down.
//
// It never prints the Steam ID, the account name, or anything else that identifies
// the account, and it never calls SteamAPI_RestartAppIfNecessary, which could ask
// Steam to launch the game.

#include <dlfcn.h>
#include <stdbool.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <sys/sysctl.h>
#include <unistd.h>

// The Steamworks flat API: C linkage, interface pointers are opaque.
typedef char SteamErrMsg[1024];
typedef int (*init_flat_fn)(SteamErrMsg *);
typedef void (*shutdown_fn)(void);
typedef void *(*accessor_fn)(void);
typedef bool (*bool_fn)(void *);
typedef uint64_t (*u64_fn)(void *);
typedef uint32_t (*u32_fn)(void *);
typedef bool (*subscribed_app_fn)(void *, uint32_t);

static const char *init_results[] = {"OK", "failed (generic)", "no Steam client", "version mismatch"};

static void *need(void *lib, const char *name) {
    void *f = dlsym(lib, name);
    if (!f) {
        printf("result: failed: %s not exported\n", name);
        exit(2);
    }
    return f;
}

int main(int argc, char **argv) {
    if (argc != 2) {
        fprintf(stderr, "usage: %s <libsteam_api.dylib>\n", argv[0]);
        return 1;
    }
    int translated = 0;
    size_t size = sizeof translated;
    sysctlbyname("sysctl.proc_translated", &translated, &size, NULL, 0);
#if defined(__x86_64__)
    printf("arch: x86_64%s\n", translated ? " (Rosetta 2)" : "");
#else
    printf("arch: arm64\n");
#endif
    const char *app = getenv("SteamAppId");
    printf("declared app: %s\n", app ? app : "(none)");

    void *lib = dlopen(argv[1], RTLD_NOW | RTLD_LOCAL);
    if (!lib) {
        printf("result: failed: dlopen: %s\n", dlerror());
        return 2;
    }
    init_flat_fn init = (init_flat_fn)need(lib, "SteamAPI_InitFlat");
    shutdown_fn shutdown = (shutdown_fn)need(lib, "SteamAPI_Shutdown");

    SteamErrMsg message = {0};
    int status = init(&message);
    printf("init: %s%s%s\n", status >= 0 && status <= 3 ? init_results[status] : "unknown result",
           message[0] ? ": " : "", message);
    if (status != 0) {
        printf("result: refused\n");
        return 3;
    }

    void *user = ((accessor_fn)need(lib, "SteamAPI_SteamUser_v024"))();
    void *apps = ((accessor_fn)need(lib, "SteamAPI_SteamApps_v009"))();
    void *utils = ((accessor_fn)need(lib, "SteamAPI_SteamUtils_v011"))();
    if (!user || !apps || !utils) {
        printf("result: failed: an interface accessor returned null\n");
        shutdown();
        return 2;
    }
    // BLoggedOn reports the client's connection to the Steam servers. Give it up to
    // ten seconds, dispatching callbacks meanwhile, as a game's main loop would.
    bool_fn logged_on_fn = (bool_fn)need(lib, "SteamAPI_ISteamUser_BLoggedOn");
    shutdown_fn run_callbacks = (shutdown_fn)need(lib, "SteamAPI_RunCallbacks");
    bool logged_on = logged_on_fn(user);
    int waited_ms = 0;
    for (; !logged_on && waited_ms < 10000; waited_ms += 100) {
        run_callbacks();
        usleep(100 * 1000);
        logged_on = logged_on_fn(user);
    }
    uint64_t id = ((u64_fn)need(lib, "SteamAPI_ISteamUser_GetSteamID"))(user);
    uint32_t seen_app = ((u32_fn)need(lib, "SteamAPI_ISteamUtils_GetAppID"))(utils);
    bool subscribed = ((bool_fn)need(lib, "SteamAPI_ISteamApps_BIsSubscribed"))(apps);
    bool holds_730 = ((subscribed_app_fn)need(lib, "SteamAPI_ISteamApps_BIsSubscribedApp"))(apps, 730);

    // CSteamID: account type in bits 52-55 (1 = individual), universe in 56-63 (1 = public).
    unsigned type = (unsigned)((id >> 52) & 0xf), universe = (unsigned)(id >> 56);
    printf("connected to Steam: %s (after %d ms)\n", logged_on ? "yes" : "no", waited_ms);
    printf("account: %s, universe %s\n", type == 1 ? "individual" : "not individual",
           universe == 1 ? "public" : "other");
    printf("app the client sees: %u\n", seen_app);
    printf("subscribed to the declared app: %s\n", subscribed ? "yes" : "no");
    printf("holds app 730: %s\n", holds_730 ? "yes" : "no");
    shutdown();
    printf("result: %s\n", seen_app == 730 && holds_730 ? "served as app 730" : "answered, but not as app 730");
    return 0;
}
