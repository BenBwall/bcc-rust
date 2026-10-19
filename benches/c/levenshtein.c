/* Levenshtein edit distance by dynamic programming over two rows, between
 * pseudo-random strings over a small alphabet, repeated for several pairs.
 * A two-level loop nest with min-of-three and byte compares. */
int printf(const char *, ...);

#ifdef BENCH_REDUCED
#define LENGTH 120
#define PAIRS 3
#define EXPECT 3272150562u
#else
#define LENGTH 2600
#define PAIRS 20
#define EXPECT 2504843022u
#endif

static unsigned char left[LENGTH];
static unsigned char right[LENGTH];
static int previous[LENGTH + 1];
static int current[LENGTH + 1];

static int minimum(int a, int b) { return a < b ? a : b; }

static int distance(void) {
  for (int j = 0; j <= LENGTH; j++) previous[j] = j;
  for (int i = 1; i <= LENGTH; i++) {
    current[0] = i;
    for (int j = 1; j <= LENGTH; j++) {
      int substitution = previous[j - 1] + (left[i - 1] != right[j - 1]);
      int edit = minimum(previous[j] + 1, current[j - 1] + 1);
      current[j] = minimum(substitution, edit);
    }
    for (int j = 0; j <= LENGTH; j++) previous[j] = current[j];
  }
  return previous[LENGTH];
}

int main(void) {
  unsigned x = 777u;
  unsigned checksum = 0;
  int total = 0;
  for (int pair = 0; pair < PAIRS; pair++) {
    for (int i = 0; i < LENGTH; i++) {
      x = x * 1664525u + 1013904223u;
      left[i] = (unsigned char)('a' + ((x >> 24) & 3));
      x = x * 1664525u + 1013904223u;
      right[i] = (unsigned char)('a' + ((x >> 24) & 3));
    }
    int d = distance();
    total += d;
    checksum = checksum * 1000003u + (unsigned)d;
  }
  printf("levenshtein: total %d checksum %u\n", total, checksum);
  return checksum == EXPECT ? 0 : 1;
}
