/* The transcendental long double libm functions on values of every x87
   encoding class, printing the raw result bits. glibc computes sinl, cosl,
   tanl, asinl, the hyperbolic and error functions, cbrtl, hypotl and fmal in C on
   long double arithmetic, and the exponentials, logarithms, powl and the
   arctangents with the x87 instructions f2xm1, fyl2x, fyl2xp1 and fpatan;
   the interpreter must print glibc's bits. powl of an invalid exponent, and
   of a non-canonical base to a small integer power, are left out: glibc 2.39
   and 2.43 differ there (long_double_modern_glibc.c pins the second). */
int printf(const char *, ...);
void *memcpy(void *, const void *, unsigned long);
long double sinl(long double), cosl(long double), tanl(long double);
long double asinl(long double), acosl(long double), atanl(long double);
long double atan2l(long double, long double);
long double expl(long double), exp2l(long double), expm1l(long double);
long double logl(long double), log2l(long double), log10l(long double), log1pl(long double);
long double powl(long double, long double);
long double sinhl(long double), coshl(long double), tanhl(long double);
long double asinhl(long double), acoshl(long double), atanhl(long double);
long double cbrtl(long double), hypotl(long double, long double);
long double fmal(long double, long double, long double);
long double erfl(long double), erfcl(long double);

static void bits(long double value)
{
  unsigned char b[16];
  memcpy(b, &value, 16);
  printf(" %02x%02x_%02x%02x%02x%02x%02x%02x%02x%02x", b[9], b[8], b[7], b[6], b[5], b[4], b[3],
         b[2], b[1], b[0]);
}

static long double make(unsigned short exponent, unsigned long long significand)
{
  long double value = 0;
  unsigned char b[16] = {0};
  memcpy(b, &significand, 8);
  memcpy(b + 8, &exponent, 2);
  memcpy(&value, b, 16);
  return value;
}

static int invalid(long double value)
{
  unsigned char b[16];
  memcpy(b, &value, 16);
  unsigned exponent = (b[9] & 0x7f) << 8 | b[8];
  return exponent != 0 && !(b[7] & 0x80);
}

/* Whether powl(base, power) is one glibc 2.43's rewritten small-integer
   path (y in [-3, 3]) computes differently from 2.39: a pseudo-denormal or
   invalid base to such a power. */
static int version_dependent(long double base, long double power)
{
  unsigned char b[16];
  memcpy(b, &base, 16);
  unsigned exponent = (b[9] & 0x7f) << 8 | b[8];
  int noncanonical = invalid(base) || (exponent == 0 && (b[7] & 0x80));
  for (int n = -3; n <= 3; n++)
    if (power == n)
      return noncanonical;
  return 0;
}

int main(void)
{
  static const unsigned short exponents[] = {
      0x3fff, 0x3ffe, 0x4000, 0x3ffe, 0x3ffe, 0x4000, 0x4002, 0x4005, 0x400c, 0x400d, 0x403e,
      0x4020, 0x7ffe, 0x3fdf, 0x3fbd, 0x0001, 0x0000, 0x0000, 0x0000, 0x7fff, 0x7fff, 0x7fff,
      0x7fff, 0x3fff, 0x0000, 0x4040, 0x3ffd, 0x3ffb, 0x4013, 0x4400};
  static const unsigned long long significands[] = {
      0x8000000000000000ULL, 0x8000000000000000ULL, 0xc90fdaa22168c235ULL, 0xc90fdaa22168c234ULL,
      0xb504f333f9de6484ULL, 0x9000000000000000ULL, 0xa000000000000000ULL, 0xc800000000000000ULL,
      0xb17217f7d1cf79abULL, 0x8000000000000000ULL, 0xffffffffffffffffULL, 0x8000000000000001ULL,
      0xffffffffffffffffULL, 0x8000000000000000ULL, 0xc000000000000000ULL, 0x8000000000000000ULL,
      0x0000000000000001ULL, 0x0000000000000000ULL, 0x8000000000000001ULL, 0x8000000000000000ULL,
      0xc000000000000000ULL, 0x8000000000000001ULL, 0x4000000000000000ULL, 0x4000000000000000ULL,
      0x4000000000000000ULL, 0x4000000000000001ULL, 0xa3d70a3d70a3d70aULL, 0xfedcba9876543210ULL,
      0x9a0e4d3c2b1a0f1eULL, 0xc90fdaa22168c235ULL};
  enum { values = sizeof exponents / sizeof exponents[0] };
  for (int i = 0; i < 2 * values; i++) {
    unsigned short exponent = exponents[i % values] | (i >= values ? 0x8000 : 0);
    long double x = make(exponent, significands[i % values]);
    printf("%d %04x trig", i, exponent);
    bits(sinl(x));
    bits(cosl(x));
    bits(tanl(x));
    bits(asinl(x));
    bits(acosl(x));
    bits(atanl(x));
    printf("\n  exp");
    bits(expl(x));
    bits(exp2l(x));
    bits(expm1l(x));
    printf("\n  log");
    bits(logl(x));
    bits(log2l(x));
    bits(log10l(x));
    bits(log1pl(x));
    printf("\n  hyperbolic");
    bits(sinhl(x));
    bits(coshl(x));
    bits(tanhl(x));
    bits(asinhl(x));
    bits(acoshl(x));
    bits(atanhl(x));
    printf("\n  algebraic");
    bits(cbrtl(x));
    bits(hypotl(x, 3.0L));
    bits(fmal(x, 3.0L, -1.0L));
    bits(erfl(x));
    bits(erfcl(x));
    printf("\n  binary");
    for (int j = 0; j < values; j += 3) {
      long double y = make(exponents[j], significands[j]);
      bits(atan2l(x, y));
      bits(atan2l(y, x));
      bits(hypotl(x, y));
      if (!invalid(y) && !version_dependent(x, y))
        bits(powl(x, y));
      if (!invalid(x) && !version_dependent(y, x))
        bits(powl(y, x));
    }
    printf("\n");
  }
  /* Powers: small integers, halves, huge and tiny exponents. */
  static const long double bases[] = {-8.0L, -2.0L, -0.5L, 0.5L, 2.0L, 10.0L, 1e100L, 1e-100L};
  static const long double powers[] = {-3.0L, -2.0L, -1.0L, 1.0L, 2.0L,   3.0L,
                                       0.5L,  -0.5L, 4.0L,  1e30L, -1e30L, 1e-30L};
  for (int i = 0; i < 8; i++) {
    printf("pow %d", i);
    for (int j = 0; j < 12; j++)
      bits(powl(bases[i], powers[j]));
    printf("\n");
  }
  /* Exact and halfway fused multiply-adds. */
  printf("fma");
  bits(fmal(1.0L + 0x1p-63L, 1.0L + 0x1p-63L, -1.0L));
  bits(fmal(1.0L + 0x1p-63L, 1.0L - 0x1p-64L, -1.0L));
  bits(fmal(0x1p-16000L, 0x1p-445L, 0x1p-16440L));
  bits(fmal(0x1p16000L, 0x1p383L, -0x1p16383L));
  bits(fmal(3.0L, 1.0L / 3.0L, -1.0L));
  bits(fmal(-0.0L, 5.0L, 0.0L));
  printf("\n");
  return 0;
}
