#warning shown
#pragma clang diagnostic fatal "-Wundef"
#if FIRST
#endif
// A fatal error ends the compilation: nothing else is reported or counted.
#if SECOND
#endif
#error not shown
