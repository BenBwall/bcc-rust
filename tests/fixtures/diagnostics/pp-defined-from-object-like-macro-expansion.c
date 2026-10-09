/* An object-like macro that produces `defined` is evaluated as GCC and
   Clang do, with the default -Wexpansion-to-defined warning. */
#define FEATURE
#define HAS_FEATURE defined(FEATURE)
#if HAS_FEATURE
int feature;
#endif
