/* conj of a real argument is that of a complex number with imaginary part
 * +0, so the result's imaginary part is -0 (C99 7.22p6, 7.3.9.3), for the
 * standard types, integers and, where bcc provides them, the _FloatN types.
 * The exit status counts the results that keep +0. */
#include <tgmath.h>
#define KEEPS_POSITIVE_ZERO(x) (!__builtin_signbit(__imag__ conj(x)))
int main(void) {
  volatile float f = 1;
  volatile double d = 1;
  volatile long double l = 1;
  volatile int i = 1;
  int wrong = KEEPS_POSITIVE_ZERO(f) + KEEPS_POSITIVE_ZERO(d) + KEEPS_POSITIVE_ZERO(l)
              + KEEPS_POSITIVE_ZERO(i);
#ifdef __FLT32_MANT_DIG__
  volatile _Float32 f32 = 1;
  volatile _Float64 f64 = 1;
  volatile _Float32x f32x = 1;
  wrong += KEEPS_POSITIVE_ZERO(f32) + KEEPS_POSITIVE_ZERO(f64) + KEEPS_POSITIVE_ZERO(f32x);
#endif
#ifdef __FLT64X_MANT_DIG__
  volatile _Float64x f64x = 1;
  wrong += KEEPS_POSITIVE_ZERO(f64x);
#endif
  return wrong;
}
