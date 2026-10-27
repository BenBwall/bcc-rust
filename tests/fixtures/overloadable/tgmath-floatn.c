/* <tgmath.h> under bcc's GCC identity, whose _FloatN types are distinct:
 * each takes the functions of the standard type with its format. */
#include <tgmath.h>
#define TYPE(expression, type) _Static_assert(__builtin_types_compatible_p(__typeof__(expression),type), #expression)
TYPE(sqrt((_Float32)1), float);
TYPE(sqrt(1.0f64), double);
TYPE(sqrt(1.0f32x), double);
TYPE(pow(1.0f32, 2.0f32), float);
TYPE(pow(1.0f32, 2), double);
TYPE(pow(1.0f32, 2.0f64), double);
TYPE(fabs(-1.0f64), double);
TYPE(fabs((_Complex _Float32)1), float);
TYPE(sin((_Complex _Float64)1), double _Complex);
TYPE(pow((_Complex _Float32)1, 2.0f32), float _Complex);
TYPE(cimag((_Complex _Float32x)1), double);
TYPE(conj(1.0f32), float _Complex);
TYPE(fma(1.0f32, 2.0f32, 3.0f32), float);
TYPE(lrint(1.0f64), long);
TYPE(nexttoward(1.0f32, 2.0L), float);
#ifdef __FLT64X_MANT_DIG__
TYPE(sqrt(1.0f64x), long double);
TYPE(atan2(1.0f64x, 2.0), long double);
TYPE(cproj((_Complex _Float64x)1), long double _Complex);
#endif
_Float32 root(_Float32 x) { return sqrt(x); }
