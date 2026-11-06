// A pragma does not bring a warning out of a system header, even an `error`,
// though GCC's own permerror comes out unless a pragma disables it; and a
// pragma in a system header outlasts it.
#pragma GCC diagnostic error "-Wundef"
#include <pragma-noisy.h>
#if QUIET_AFTER_HEADER
#endif
#pragma GCC diagnostic ignored "-Wint-conversion"
#define system_conversion second_system_conversion
#include <pragma-noisy.h>
#pragma GCC diagnostic warning "-Wundef"
#if LOUD
#endif
