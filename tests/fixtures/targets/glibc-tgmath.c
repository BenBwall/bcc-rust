#define _GNU_SOURCE 1
#include <tgmath.h>
#include <stdlib.h>
_Static_assert(__HAVE_FLOAT128 == 1, "glibc binary128 enabled");
_Static_assert(sizeof(_Float128) == 16, "glibc typedef");
_Static_assert(__builtin_types_compatible_p(_Float128, __float128), "glibc identity");
_Static_assert(__builtin_types_compatible_p(_Float32, float), "Float32 typedef");
_Static_assert(__builtin_types_compatible_p(_Float64, double), "Float64 typedef");
_Static_assert(__builtin_types_compatible_p(_Float32x, double), "Float32x typedef");
_Static_assert(__builtin_types_compatible_p(_Float64x, long double), "Float64x typedef");
_Static_assert(_Generic(sqrt(1.0f), float: 1, default: 0), "float selection");
_Static_assert(_Generic(sqrt(1), double: 1, default: 0), "integer promotion");
_Static_assert(_Generic(sqrt(1.0L), long double: 1, default: 0), "long double selection");
_Static_assert(_Generic(sqrt(1.0q), _Float128: 1, default: 0), "binary128 selection");
_Static_assert(_Generic(pow(1.0f, 2), double: 1, default: 0), "mixed promotion");
_Static_assert(_Generic(pow(1.0q, 2.0L), _Float128: 1, default: 0), "binary128 rank");
_Static_assert(_Generic(sin(1.0fi), _Complex float: 1, default: 0), "complex selection");
_Static_assert(_Generic(fabs((__CFLOAT128)0), _Float128: 1, default: 0), "complex result");
_Static_assert(_Generic(strtof128("1", 0), _Float128: 1, default: 0), "stdlib Float128");
_Float128 parse_quad(char *text) { return sqrt(strtof128(text, 0)); }
_Float128 quad_infinity=HUGE_VAL_F128;
_Float128 quad_nan=__builtin_nanf128("");
_Float128 quad_absolute=__builtin_fabsf128(-1.0q);
_Float128 quad_sign=__builtin_copysignf128(1.0q,-2.0q);
_Static_assert(_Generic(__builtin_fabsf128(1.0q),_Float128:1,default:0),"quad builtin");
