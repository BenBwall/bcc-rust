/* A three-point smoothing stencil and a polynomial fold through pointer
 * parameters, ping-ponging between two buffers. Loads of neighbouring
 * elements, pointer arguments that may alias as far as the compiler knows,
 * and simple counted loops. */
int printf(const char *, ...);

#ifdef BENCH_REDUCED
#define N 2000
#define ROUNDS 4
#define EXPECT 245794349u
#else
#define N 20000
#define ROUNDS 6000
#define EXPECT 2215033013u
#endif

static int a[N];
static int b[N];

static void smooth(int *dst, const int *src, int n) {
  dst[0] = src[0];
  for (int i = 1; i < n - 1; i++) dst[i] = (src[i - 1] + 2 * src[i] + src[i + 1]) / 4;
  dst[n - 1] = src[n - 1];
}

static void inject(int *v, int n, unsigned salt) {
  for (int i = 0; i < n; i += 97) v[i] += (int)((salt ^ (unsigned)i) & 255u);
}

static unsigned fold(const int *v, int n) {
  unsigned sum = 0;
  for (int i = 0; i < n; i++) sum = sum * 3u + (unsigned)v[i];
  return sum;
}

int main(void) {
  unsigned x = 777u;
  for (int i = 0; i < N; i++) {
    x = x * 1664525u + 1013904223u;
    a[i] = (int)(x >> 22);
  }
  unsigned checksum = 0;
  for (int round = 0; round < ROUNDS; round++) {
    smooth(b, a, N);
    inject(b, N, (unsigned)round * 2654435761u);
    smooth(a, b, N);
    if (round % 64 == 0) checksum ^= fold(a, N);
  }
  checksum ^= fold(a, N);
  printf("stencil: checksum %u\n", checksum);
  return checksum == EXPECT ? 0 : 1;
}
