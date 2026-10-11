/* Longest Collatz chain below LIMIT, found by walking every chain. `long
 * long` because `long` is 32 bits on Windows. Data-dependent branches, 64-bit
 * arithmetic and no memory traffic. */
int printf(const char *, ...);

#ifdef BENCH_REDUCED
#define LIMIT 6000
#define EXPECT 4110407u
#else
#define LIMIT 3000000
#define EXPECT 2676521059u
#endif

static unsigned chain_length(long long n) {
  unsigned steps = 1;
  while (n != 1) {
    if (n & 1)
      n = 3 * n + 1;
    else
      n = n / 2;
    steps++;
  }
  return steps;
}

int main(void) {
  unsigned best = 0;
  unsigned best_start = 0;
  unsigned long long total = 0;
  for (unsigned start = 1; start < LIMIT; start++) {
    unsigned length = chain_length(start);
    total += length;
    if (length > best) {
      best = length;
      best_start = start;
    }
  }
  unsigned checksum = best_start * 977u + best * 13u + (unsigned)total;
  printf("collatz: start %u length %u total %llu checksum %u\n", best_start, best,
         total, checksum);
  return checksum == EXPECT ? 0 : 1;
}
