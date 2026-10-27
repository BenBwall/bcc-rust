/* x87 long double programs whose output every arm must print alike: native
   clang, bcc through LLVM, bcc's native back end and the interpreter. Inputs
   are volatile so that no compiler folds them with its own NaN rules. */
typedef __builtin_va_list va_list;
int printf(const char *, ...);
int snprintf(char *, unsigned long, const char *, ...);
long double sqrtl(long double);
long double fabsl(long double);
long double floorl(long double);
long double ceill(long double);
long double truncl(long double);
long double roundl(long double);
long double rintl(long double);
long double nearbyintl(long double);
long lrintl(long double);
long long llroundl(long double);
long double fmodl(long double, long double);
long double remainderl(long double, long double);
long double remquol(long double, long double, int *);
long double frexpl(long double, int *);
long double ldexpl(long double, int);
long double scalbnl(long double, int);
long double modfl(long double, long double *);
long double nextafterl(long double, long double);
long double copysignl(long double, long double);
long double fmaxl(long double, long double);
long double fminl(long double, long double);
long double fdiml(long double, long double);
long double logbl(long double);
int ilogbl(long double);
long double nanl(const char *);
double nexttoward(double, long double);

static volatile long double inputs[14];

static void fill(void)
{
  volatile long double zero = 0.0L, one = 1.0L, three = 3.0L;
  inputs[0] = one;
  inputs[1] = -2.5L;
  inputs[2] = one / three;
  inputs[3] = 1e4000L;
  inputs[4] = 1e-4940L;
  inputs[5] = zero;
  inputs[6] = -zero;
  inputs[7] = 18446744073709551615.0L;
  inputs[8] = one / zero;
  inputs[9] = zero / zero;
  inputs[10] = 0.1L;
  inputs[11] = -123456789.987654321L;
  inputs[12] = 3.3621031431120935063e-4932L;
  inputs[13] = 0.5L;
}

/* Sums its long double arguments through va_arg. */
static long double sum(int count, ...)
{
  va_list list;
  long double total = 0;
  __builtin_va_start(list, count);
  for (int i = 0; i < count; i++)
    total += __builtin_va_arg(list, long double);
  __builtin_va_end(list);
  return total;
}

int main(void)
{
  fill();
  for (int i = 0; i < 14; i++) {
    long double x = inputs[i];
    printf("%d: %La %.21Lg %Le %.3Lf %#Lg\n", i, x, x, x, x, x);
    printf("  double %a float %a", (double)x, (double)(float)x);
    if (x == x && fabsl(x) < 9e18L)
      printf(" ll %lld", (long long)x);
    if (x >= 0 && x < 1.8e19L)
      printf(" ull %llu", (unsigned long long)x);
    printf("\n  sqrt %La floor %La ceil %La trunc %La round %La rint %La\n", sqrtl(x),
           floorl(x), ceill(x), truncl(x), roundl(x), rintl(x));
    int exponent = 0;
    long double fraction = frexpl(x, &exponent);
    long double integral = 0;
    long double part = modfl(x, &integral);
    printf("  frexp %La %d ldexp %La %La modf %La %La logb %La nearbyint %La\n", fraction,
           exponent, ldexpl(x, 70), scalbnl(x, -16400), part, integral, logbl(x), nearbyintl(x));
    if (x == x && fabsl(x) < 9e18L)
      printf("  lrint %ld llround %lld ilogb %d\n", lrintl(x), llroundl(x), ilogbl(x));
    printf("  next %La %La copysign %La\n", nextafterl(x, 1e30L), nextafterl(x, -1e30L),
           copysignl(2.0L, x));
    for (int j = 0; j < 14; j++) {
      long double y = inputs[j];
      int quotient = 0;
      long double remainder = remquol(x, y, &quotient);
      printf("  %d %d: %La %La %La %La | %La %La %La %d | %La %La %La | %d%d%d%d%d%d\n", i, j,
             x + y, x - y, x * y, x / y, fmodl(x, y), remainderl(x, y), remainder,
             x == x && y == y ? quotient : 0, fmaxl(x, y), fminl(x, y), fdiml(x, y), x < y,
             x <= y, x == y, x != y, x > y, x >= y);
    }
  }
  printf("sum %La\n", sum(4, inputs[0], inputs[2], inputs[10], inputs[13]));
  printf("nan %La %La\n", nanl(""), nanl("0x1234"));
  printf("nexttoward %a %a\n", nexttoward(1.0, 2.0L), nexttoward(1.0, 1.0L - 1e-19L));
  char text[64];
  int length = snprintf(text, sizeof text, "%+.4Le|%-12.2Lf|%012La", inputs[2], inputs[1],
                        inputs[0]);
  printf("%d %s\n", length, text);
  static const long long integers[] = {-3, 0, 1, -9223372036854775807LL - 1,
                                       9223372036854775807LL, 123456789012345678LL};
  for (int i = 0; i < 6; i++) {
    long long value = integers[i];
    unsigned long long big = (unsigned long long)value * 3;
    long double wide = value, unsigned_wide = big;
    printf("%lld %La %llu %La\n", value, wide, big, unsigned_wide);
  }
  return 0;
}
