/* Bit manipulation over a stream of pseudo-random words: population count
 * by clearing the lowest set bit, bit reversal in a counted 32-step loop,
 * binary GCD and a branchy parity fold. Shifts, masks, small counted loops
 * and data-dependent branches. */
int printf(const char *, ...);

#ifdef BENCH_REDUCED
#define COUNT 3000
#define EXPECT 355028908u
#else
#define COUNT 2000000
#define EXPECT 2766843816u
#endif

static int popcount(unsigned v) {
  int n = 0;
  while (v) {
    v &= v - 1;
    n++;
  }
  return n;
}

static unsigned reverse(unsigned v) {
  unsigned r = 0;
  for (int i = 0; i < 32; i++) {
    r = (r << 1) | (v & 1u);
    v >>= 1;
  }
  return r;
}

static unsigned gcd(unsigned a, unsigned b) {
  if (a == 0) return b;
  if (b == 0) return a;
  int shift = 0;
  while (((a | b) & 1u) == 0) {
    a >>= 1;
    b >>= 1;
    shift++;
  }
  while ((a & 1u) == 0) a >>= 1;
  do {
    while ((b & 1u) == 0) b >>= 1;
    if (a > b) {
      unsigned t = a;
      a = b;
      b = t;
    }
    b -= a;
  } while (b != 0);
  return a << shift;
}

int main(void) {
  unsigned x = 88172645u;
  unsigned checksum = 0;
  unsigned previous = 1;
  for (int i = 0; i < COUNT; i++) {
    x ^= x << 13;
    x ^= x >> 17;
    x ^= x << 5;
    checksum += (unsigned)popcount(x);
    checksum ^= reverse(x);
    checksum += gcd(x & 0xffffu, previous & 0xffffu);
    if (popcount(x & 0xff) & 1) checksum = (checksum << 1) | (checksum >> 31);
    previous = x;
  }
  printf("bits: checksum %u\n", checksum);
  return checksum == EXPECT ? 0 : 1;
}
