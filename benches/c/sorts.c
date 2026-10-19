/* Quicksort and top-down mergesort of the same pseudo-random unsigned array.
 * Recursion, pointer arguments, compares, swaps and a scratch buffer; both
 * results are checked against each other. */
int printf(const char *, ...);

#ifdef BENCH_REDUCED
#define N 5000
#define EXPECT 2382965148u
#else
#define N 1500000
#define EXPECT 1613549008u
#endif

static unsigned by_quick[N];
static unsigned by_merge[N];
static unsigned scratch[N];

static void quicksort(unsigned *v, int lo, int hi) {
  while (lo < hi) {
    unsigned pivot = v[lo + (hi - lo) / 2];
    int i = lo;
    int j = hi;
    while (i <= j) {
      while (v[i] < pivot) i++;
      while (v[j] > pivot) j--;
      if (i <= j) {
        unsigned t = v[i];
        v[i] = v[j];
        v[j] = t;
        i++;
        j--;
      }
    }
    /* Recurse into the smaller part, loop on the larger one. */
    if (j - lo < hi - i) {
      quicksort(v, lo, j);
      lo = i;
    } else {
      quicksort(v, i, hi);
      hi = j;
    }
  }
}

static void mergesort(unsigned *v, unsigned *tmp, int lo, int hi) {
  if (hi - lo < 2) return;
  int mid = lo + (hi - lo) / 2;
  mergesort(v, tmp, lo, mid);
  mergesort(v, tmp, mid, hi);
  int i = lo;
  int j = mid;
  int k = lo;
  while (i < mid && j < hi) tmp[k++] = v[j] < v[i] ? v[j++] : v[i++];
  while (i < mid) tmp[k++] = v[i++];
  while (j < hi) tmp[k++] = v[j++];
  for (k = lo; k < hi; k++) v[k] = tmp[k];
}

int main(void) {
  unsigned x = 424242u;
  for (int i = 0; i < N; i++) {
    x = x * 1664525u + 1013904223u;
    by_quick[i] = x;
    by_merge[i] = x;
  }
  quicksort(by_quick, 0, N - 1);
  mergesort(by_merge, scratch, 0, N);
  int agree = 1;
  int ordered = 1;
  unsigned checksum = 0;
  for (int i = 0; i < N; i++) {
    if (by_quick[i] != by_merge[i]) agree = 0;
    if (i > 0 && by_quick[i - 1] > by_quick[i]) ordered = 0;
    checksum = checksum * 33u + by_quick[i];
  }
  printf("sorts: agree %d ordered %d checksum %u\n", agree, ordered, checksum);
  return agree && ordered && checksum == EXPECT ? 0 : 1;
}
