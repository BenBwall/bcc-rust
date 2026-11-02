/* conj of a real argument is that of a complex number with imaginary part
 * +0 (C99 7.22p6, 7.3.9.3). Under bcc's GCC identity the result's imaginary
 * part is -0, as GCC's and glibc's conj give, for the standard types,
 * integers and the _FloatN types; otherwise it stays +0, as the pinned
 * Clang's header gives. The exit status counts the results of the other
 * sign. */
#include <tgmath.h>
#if defined(__GNUC__) && !defined(__clang__)
#define NEGATIVE 1
#else
#define NEGATIVE 0
#endif
/* The sign of a zero, through 1/x, which every identity has (TCC has no
 * __builtin_signbit). */
#define WRONG(x) (1 / (double)(__imag__ conj(x)) < 0 ? !NEGATIVE : NEGATIVE)
int main(void) {
  volatile float f = 1;
  volatile double d = 1;
  volatile long double l = 1;
  volatile int i = 1;
  int wrong = WRONG(f) + WRONG(d) + WRONG(l) + WRONG(i);
#ifdef __FLT32_MANT_DIG__
  volatile _Float32 f32 = 1;
  volatile _Float64 f64 = 1;
  volatile _Float32x f32x = 1;
  wrong += WRONG(f32) + WRONG(f64) + WRONG(f32x);
#endif
#ifdef __FLT64X_MANT_DIG__
  volatile _Float64x f64x = 1;
  wrong += WRONG(f64x);
#endif
  return wrong;
}
