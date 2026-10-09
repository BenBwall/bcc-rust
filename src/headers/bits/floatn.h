/* bcc's glibc binary128 capability bridge.
 * GCC 4.2.1 identity is retained; the actual type capability is newer.
 * The library still owns its other _FloatN typedefs and feature decisions.
 */
#ifndef __BCC_BITS_FLOATN_H
#define __BCC_BITS_FLOATN_H

#include_next <bits/floatn.h>

#if defined __GLIBC__ && defined __SIZEOF_FLOAT128__ && !__HAVE_FLOAT128
#undef __HAVE_FLOAT128
#define __HAVE_FLOAT128 1
#undef __HAVE_DISTINCT_FLOAT128
#define __HAVE_DISTINCT_FLOAT128 1

typedef __float128 _Float128;
typedef _Complex __float128 __bcc_cfloat128;
#define __CFLOAT128 __bcc_cfloat128
#define __f128(x) x##q

#define __builtin_huge_valf128() ((_Float128) __builtin_huge_val())
/* Clang-style fabsf128/copy-sign builtins are available directly. */
#define __builtin_inff128() ((_Float128) __builtin_inf())
#define __builtin_nanf128(x) ((_Float128) __builtin_nan(x))
#define __builtin_nansf128(x) ((_Float128) __builtin_nans(x))
#define __builtin_signbitf128 __signbitf128
#endif

#endif
