// Needs the override library, then the base library, found through $ORIGIN.
int versioned(void);
int shared_name(void);
int overridable(void);
int base_value(void);
int base_calls_shared(void);
__attribute__((weak)) int scope_absent(void);

int user_versioned(void) { return versioned(); }
int user_shared(void) { return shared_name(); }
int user_overridable(void) { return overridable(); }
int user_base_calls_shared(void) { return base_calls_shared(); }
int user_base_value(void) { return base_value(); }
int user_absent_is_null(void) { return scope_absent == 0; }
