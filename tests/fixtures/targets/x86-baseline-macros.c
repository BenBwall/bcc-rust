#if !defined(__MMX__) || !defined(__SSE__) || !defined(__SSE2__)
#error baseline x86-64 intrinsic features must be advertised
#endif
#if !defined(__SSE_MATH__) || !defined(__SSE2_MATH__)
#error baseline scalar floating arithmetic uses SSE
#endif

/* MinGW's intrin.h fallback would conflict with mmintrin.h. */
#include <mmintrin.h>
#ifndef __MMX__
typedef union __m64 { char v[7]; } __m64;
#endif
_Static_assert(sizeof(__m64) == 8, "MMX vector size");
