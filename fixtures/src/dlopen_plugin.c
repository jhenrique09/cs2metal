// A plugin loaded at run time, as the engine loads its modules.
#include <stdio.h>

static int loads;

__attribute__((constructor)) static void on_load(void) { loads++; }

int plugin_answer(int x) { return x * 2 + loads; }

const char *plugin_name(void) { return "dlopen plugin"; }
