#pragma clang diagnostic
#pragma clang diagnostic bogus
#pragma clang diagnostic ignored
#pragma clang diagnostic ignored -Wundef
#pragma clang diagnostic ignored "-Wnosuch"
#pragma clang diagnostic ignored "Wundef"
#pragma clang diagnostic ignored "-Werror=undef"
#pragma clang diagnostic warning L"-Wundef"
#pragma clang diagnostic pop junk
#pragma clang diagnostic push junk
#pragma clang diagnostic pop
#pragma clang diagnostic pop
// A real option bcc has no group for stays quiet; GCC's alone do not.
#pragma clang diagnostic ignored "-Wdouble-promotion"
#pragma clang diagnostic ignored "-Wstringop-overflow"
#pragma clang diagnostic ignored "-Wno-undef"
#pragma clang diagnostic ignored "-Rpass"
_Pragma("clang diagnostic ignored \"-Wnosuch\"")
// Clang reads GCC's namespace too.
#pragma GCC diagnostic ignored "-Wundef" junk
#if LOUD
#endif
#pragma GCC diagnostic ignored "-Wundef"
#if QUIET
#endif
#define OPTION "-Wundef"
#pragma clang diagnostic warning OPTION
#if STILL_QUIET
#endif
#pragma clang diagnostic ignored "-Wunknown-warning-option"
#pragma clang diagnostic ignored "-Wnosuch"
#pragma clang diagnostic ignored "-Wunknown-pragmas"
#pragma clang diagnostic bogus
int declared;
