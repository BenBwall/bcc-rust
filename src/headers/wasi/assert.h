/* bcc's bundled C library for wasm32-wasip1: <assert.h>. Not guarded:
   C99 §7.2p1 redefines assert at each inclusion. */
#undef assert
#ifdef NDEBUG
#define assert(ignore) ((void)0)
#else
void __bcc_assert_fail(const char *expression, const char *file, int line, const char *function);
#define assert(expression) \
    ((expression) ? (void)0 : __bcc_assert_fail(#expression, __FILE__, __LINE__, __func__))
#endif
#if !defined(__cplusplus) && __STDC_VERSION__ >= 201112L && !defined(static_assert)
#define static_assert _Static_assert
#endif
