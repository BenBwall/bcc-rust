/* Dense multiplication of two N-by-N unsigned matrices filled by a linear
 * congruential generator, in the textbook i-j-k order. Two-dimensional
 * arrays, address arithmetic and a multiply-add inner loop. */
int printf(const char *, ...);

#ifdef BENCH_REDUCED
#define N 36
#define EXPECT 852015863u
#else
#define N 640
#define EXPECT 3275568936u
#endif

static unsigned a[N][N];
static unsigned b[N][N];
static unsigned c[N][N];

int main(void) {
  unsigned x = 12345u;
  for (int i = 0; i < N; i++) {
    for (int j = 0; j < N; j++) {
      x = x * 1664525u + 1013904223u;
      a[i][j] = x >> 20;
      x = x * 1664525u + 1013904223u;
      b[i][j] = x >> 20;
    }
  }
  for (int i = 0; i < N; i++) {
    for (int j = 0; j < N; j++) {
      unsigned sum = 0;
      for (int k = 0; k < N; k++) sum += a[i][k] * b[k][j];
      c[i][j] = sum;
    }
  }
  unsigned checksum = 0;
  for (int i = 0; i < N; i++)
    for (int j = 0; j < N; j++) checksum = checksum * 31u + c[i][j];
  printf("matmul: n %d checksum %u\n", N, checksum);
  return checksum == EXPECT ? 0 : 1;
}
