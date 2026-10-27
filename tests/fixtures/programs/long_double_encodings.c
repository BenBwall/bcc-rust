/* Every x87 long double encoding class, built from raw bytes and passed
   through the libm functions the interpreter computes, printing the raw
   result bits. glibc implements some of these with x87 instructions and some
   with C code on the representation's words, and the two treat unnormals,
   pseudo-denormals, pseudo-infinities and pseudo-NaNs differently; the
   interpreter must print glibc's bits. */
int printf(const char *, ...);
void *memcpy(void *, const void *, unsigned long);
long double frexpl(long double, int *), ldexpl(long double, int), scalbnl(long double, int);
long double scalblnl(long double, long);
long double floorl(long double), ceill(long double), truncl(long double), roundl(long double);
long double rintl(long double), nearbyintl(long double);
long lrintl(long double), lroundl(long double);
long long llroundl(long double), llrintl(long double);
long double fmodl(long double, long double), remainderl(long double, long double);
long double remquol(long double, long double, int *);
long double sqrtl(long double), fabsl(long double), copysignl(long double, long double);
long double nextafterl(long double, long double), nexttowardl(long double, long double);
long double logbl(long double), modfl(long double, long double *);
int ilogbl(long double);
long double fmaxl(long double, long double), fminl(long double, long double);
long double fdiml(long double, long double);
long double nanl(const char *);

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

int main(void)
{
  static const unsigned short exponents[] = {0x3fff, 0x0000, 0x0000, 0x7fff, 0x7fff, 0x7fff,
                                             0x7fff, 0x7fff, 0x0000, 0x4040, 0x3ffe, 0x7ffe,
                                             0x0001, 0x403e, 0x403d, 0x3ffd, 0x4000, 0x0000};
  static const unsigned long long significands[] = {
      0x4000000000000000ULL, 0x8000000000000001ULL, 0x0000000000000003ULL, 0x0000000000000000ULL,
      0x4000000000000000ULL, 0x8000000000000001ULL, 0xc000000000000001ULL, 0x8000000000000000ULL,
      0x0000000000000000ULL, 0x4000000000000001ULL, 0xc000000000000000ULL, 0xffffffffffffffffULL,
      0x8000000000000000ULL, 0xc000000000000000ULL, 0xffffffffffffffffULL, 0x8000000000000000ULL,
      0x0000000000000001ULL, 0xffffffffffffffffULL};
  volatile long double one = 1.5L, inf = 1.0L / 0.0L;
  for (int i = 0; i < 36; i++) {
    unsigned short exponent = exponents[i % 18] | (i >= 18 ? 0x8000 : 0);
    long double x = make(exponent, significands[i % 18]);
    int e = 12345, q = 12345;
    long double integral = 0;
    printf("%d %04x frexp", i, exponent);
    bits(frexpl(x, &e));
    printf(" %d ldexp", e);
    bits(ldexpl(x, 1));
    bits(ldexpl(x, -16400));
    bits(scalbnl(x, 70));
    bits(scalblnl(x, 1));
    printf("\n  round");
    bits(floorl(x));
    bits(ceill(x));
    bits(truncl(x));
    bits(roundl(x));
    bits(rintl(x));
    bits(nearbyintl(x));
    printf(" %ld %lld %lld %ld", lrintl(x), llroundl(x), llrintl(x), lroundl(x));
    printf("\n  rem");
    bits(fmodl(x, one));
    bits(remainderl(x, one));
    bits(remquol(x, one, &q));
    printf(" %d", q);
    bits(fmodl(one, x));
    bits(remainderl(one, x));
    printf("\n  next");
    bits(nextafterl(x, 0));
    bits(nextafterl(x, inf));
    bits(nextafterl(x, -inf));
    bits(nextafterl(x, x));
    bits(nexttowardl(x, one));
    printf("\n  misc");
    bits(sqrtl(x));
    bits(fabsl(x));
    bits(copysignl(one, x));
    bits(logbl(x));
    printf(" %d", ilogbl(x));
    bits(modfl(x, &integral));
    bits(integral);
    printf("\n  minmax");
    bits(fmaxl(x, one));
    bits(fminl(x, one));
    bits(fmaxl(one, x));
    bits(fminl(one, x));
    bits(fmaxl(x, x));
    bits(fdiml(x, one));
    bits(fdiml(one, x));
    printf("\n  conv %a %a\n", (double)x, (double)(float)x);
  }
  /* NaN payloads: overflow saturates, and only whole numbers count. */
  static const char *const payloads[] = {"18446744073709551616", "0x10000000000000000", "-5",
                                         " 7", "+9", "08", "0x", "0x+5", "12a", ""};
  for (int i = 0; i < 10; i++) {
    printf("nanl(\"%s\")", payloads[i]);
    bits(nanl(payloads[i]));
    printf("\n");
  }
  return 0;
}
