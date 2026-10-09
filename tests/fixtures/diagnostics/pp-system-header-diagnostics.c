#include <noisy.h>
#include "pragma-system-header.h"
/* The user's own file keeps every diagnostic, including those about
   what a system header defined. */
#define NOISY_VALUE 2
long long user_long_long;
NOISY_WIDE user_from_system_macro;
#if UNDEFINED_IN_USER_FILE
#endif
