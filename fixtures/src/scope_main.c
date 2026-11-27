// Prints what the scope fixture's library answers on Linux: the ground truth for
// the loader's own test, which calls the same functions.
#include <stdio.h>

int user_versioned(void);
int user_shared(void);
int user_overridable(void);
int user_base_calls_shared(void);
int user_base_value(void);
int user_absent_is_null(void);

int main(void) {
    printf("versioned=%d\n", user_versioned());
    printf("shared=%d\n", user_shared());
    printf("overridable=%d\n", user_overridable());
    printf("base_calls_shared=%d\n", user_base_calls_shared());
    printf("base_value=%d\n", user_base_value());
    printf("absent_is_null=%d\n", user_absent_is_null());
    return 0;
}
