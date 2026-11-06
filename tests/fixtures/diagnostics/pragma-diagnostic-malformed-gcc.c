#pragma GCC diagnostic
#pragma GCC diagnostic bogus
#pragma GCC diagnostic ignored
#pragma GCC diagnostic ignored -Wundef
#pragma GCC diagnostic ignored "-Wnosuch"
#pragma GCC diagnostic ignored "Wundef"
#pragma GCC diagnostic ignored "-Werror=undef"
#pragma GCC diagnostic ignored "-pedantic"
#pragma GCC diagnostic warning L"-Wundef"
#pragma GCC diagnostic fatal "-Wundef"
#pragma GCC diagnostic pop junk
#pragma GCC diagnostic push junk
#pragma GCC diagnostic pop
#pragma GCC diagnostic ignored_attributes
// A real option bcc has no group for stays quiet.
#pragma GCC diagnostic ignored "-Wstringop-overflow"
#pragma GCC diagnostic ignored "-Wdouble-promotion"
#pragma GCC diagnostic ignored "-Wno-undef"
_Pragma("GCC diagnostic ignored \"-Wnosuch\"")
// Clang's namespace is an unknown pragma, and does nothing.
#pragma clang diagnostic ignored "-Wundef"
#if LOUD
#endif
// Junk after the option does not stop GCC, and operands are not replaced.
#pragma GCC diagnostic ignored "-Wundef" junk
#if QUIET
#endif
#define OPTION "-Wundef"
#pragma GCC diagnostic warning OPTION
#if STILL_QUIET
#endif
// The idiom that lets a pragma name an option an older compiler lacks.
#pragma GCC diagnostic ignored "-Wpragmas"
#pragma GCC diagnostic ignored "-Wnosuch"
int declared;
