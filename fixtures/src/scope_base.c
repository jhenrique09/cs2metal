// The base library of the scope fixture: two versions of one symbol, a weak
// definition, and a call to an exported function that another module interposes.
// No C library: the loader can run it before it has one.
int base_value(void) { return 1; }

int versioned_v1(void) { return 10; }
int versioned_v2(void) { return 20; }
__asm__(".symver versioned_v1,versioned@SCOPE_1");
__asm__(".symver versioned_v2,versioned@@SCOPE_2");

__attribute__((weak)) int overridable(void) { return 100; }

int shared_name(void) { return 1000; }

// Calls through the PLT, so the definition earliest in the global scope wins.
int base_calls_shared(void) { return shared_name(); }
