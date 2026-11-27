// Loaded before the base library, so its definitions come first in the global
// scope: it interposes shared_name and overrides the weak overridable.
int shared_name(void) { return 2000; }
int overridable(void) { return 200; }
