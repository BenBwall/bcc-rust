/* Sieve of Eratosthenes over a byte array: counts the primes up to N and
 * folds them into a checksum. Integer loops and byte stores.
 * Build with -DBENCH_REDUCED for the small workload the interpreter checks. */
int printf(const char *, ...);

#ifdef BENCH_REDUCED
#define N 300000
#define EXPECT 3710313021u
#else
#define N 64000000
#define EXPECT 1894808135u
#endif

static unsigned char composite[N + 1];

int main(void) {
  unsigned count = 0;
  unsigned sum = 0;
  for (unsigned i = 2; (unsigned long long)i * i <= N; i++) {
    if (!composite[i]) {
      for (unsigned j = i * i; j <= N; j += i) composite[j] = 1;
    }
  }
  for (unsigned i = 2; i <= N; i++) {
    if (!composite[i]) {
      count++;
      sum += i;
    }
  }
  unsigned checksum = count * 31u + sum;
  printf("sieve: primes %u checksum %u\n", count, checksum);
  return checksum == EXPECT ? 0 : 1;
}
