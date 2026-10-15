/* Fannkuch-redux: for every permutation of 0..N-1, count the prefix
 * reversals until the first element is 0. Small local arrays indexed by
 * loop counters, short data-dependent loops and swaps. */
int printf(const char *, ...);

#ifdef BENCH_REDUCED
#define N 6
#define EXPECT 49010u
#else
#define N 10
#define EXPECT 73196038u
#endif

int main(void) {
  int perm[N];
  int perm1[N];
  int count[N];
  int max_flips = 0;
  int checksum = 0;
  int permutations = 0;
  for (int i = 0; i < N; i++) perm1[i] = i;
  int r = N;
  for (;;) {
    while (r != 1) {
      count[r - 1] = r;
      r--;
    }
    for (int i = 0; i < N; i++) perm[i] = perm1[i];
    int flips = 0;
    int k;
    while ((k = perm[0]) != 0) {
      int lo = 0;
      int hi = k;
      while (lo < hi) {
        int t = perm[lo];
        perm[lo] = perm[hi];
        perm[hi] = t;
        lo++;
        hi--;
      }
      flips++;
    }
    if (flips > max_flips) max_flips = flips;
    checksum += permutations % 2 == 0 ? flips : -flips;
    permutations++;
    for (;;) {
      if (r == N) {
        unsigned result = (unsigned)checksum * 1000u + (unsigned)max_flips;
        printf("fannkuch: n %d checksum %d max flips %d\n", N, checksum, max_flips);
        return result == EXPECT ? 0 : 1;
      }
      int first = perm1[0];
      for (int i = 0; i < r; i++) perm1[i] = perm1[i + 1];
      perm1[r] = first;
      count[r]--;
      if (count[r] > 0) break;
      r++;
    }
  }
}
