/* A user header found beside its includer. Diagnostics before the pragma
   remain; the rest of the file is a system header. */
#if UNDEFINED_BEFORE_PRAGMA
#endif
#pragma GCC system_header
#if UNDEFINED_AFTER_PRAGMA
#endif
long long after_pragma;
