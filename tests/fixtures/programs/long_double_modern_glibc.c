/* powl of non-canonical x87 bases (pseudo-denormals, unnormals,
   pseudo-infinities and pseudo-NaNs) to the integer powers -3 to 3, on
   which glibc 2.43 (whose results the interpreter follows) differs from
   earlier releases: its e_powl.S walks a small integral y in [-3, 3]
   directly, returning the base itself for y == 1, where 2.39 multiplied
   through a squaring loop. long_double_modern_glibc.expected records 2.43's
   output. */
int printf(const char *, ...);
void *memcpy(void *, const void *, unsigned long);
long double powl(long double, long double);

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
  static const unsigned short exponents[] = {0x0000, 0x8000, 0x3fff, 0x4040, 0x7fff, 0xffff,
                                             0x7fff, 0x0001};
  static const unsigned long long significands[] = {
      0x8000000000000001ULL, 0x8000000000000001ULL, 0x4000000000000000ULL,
      0x4000000000000001ULL, 0x0000000000000000ULL, 0x4000000000000000ULL,
      0x0000000000000001ULL, 0x4000000000000000ULL};
  for (int i = 0; i < 8; i++) {
    long double base = make(exponents[i], significands[i]);
    printf("%04x_%016llx", exponents[i], significands[i]);
    for (int n = -3; n <= 3; n++) {
      long double value = powl(base, n);
      unsigned char b[16];
      memcpy(b, &value, 16);
      printf(" %02x%02x_%02x%02x%02x%02x%02x%02x%02x%02x", b[9], b[8], b[7], b[6], b[5], b[4],
             b[3], b[2], b[1], b[0]);
    }
    printf("\n");
  }
  return 0;
}
