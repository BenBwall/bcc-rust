#include <tgmath.h>
#define TYPE(expression, type) _Static_assert(__builtin_types_compatible_p(__typeof__(expression),type), #expression)
TYPE(sin(1), double);
TYPE(sin((_Bool)1), double);
TYPE(sin((short)1), double);
TYPE(sin(1.0f), float);
TYPE(sin(1.0), double);
TYPE(sin(1.0L), long double);
TYPE(sin((float _Complex)1), float _Complex);
TYPE(sin((double _Complex)1), double _Complex);
TYPE(sin((long double _Complex)1), long double _Complex);
TYPE(pow(1.0f, 2), double);
TYPE(pow(1.0f, 2.0f), float);
TYPE(pow((float _Complex)1, 2.0), double _Complex);
TYPE(pow(1.0L, (float _Complex)2), long double _Complex);
TYPE(fabs((float _Complex)1), float);
TYPE(fabs((long double _Complex)1), long double);
TYPE(creal(1), double);
TYPE(cimag(1.0f), float);
TYPE(carg((double _Complex)1), double);
TYPE(conj(1.0f), float _Complex);
TYPE(cproj(1.0L), long double _Complex);
TYPE(fma(1.0f, 2, 3.0f), double);
TYPE(fma(1.0f, 2.0f, 3.0f), float);
TYPE(nexttoward(1.0f, 2.0L), float);
TYPE(lrint(1.0f), long);
TYPE(llround(1.0L), long long);
float side_effect(float *p) { return sin((*p)++); }
