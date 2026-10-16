/* Byte-at-a-time text processing over a generated buffer of words: word
 * splitting, FNV-1a hashing, case conversion in place and a hand-written
 * string length. Character loads, zero extensions and short inner loops. */
int printf(const char *, ...);

#ifdef BENCH_REDUCED
#define SIZE 3000
#define ROUNDS 2
#define EXPECT 242478247u
#else
#define SIZE 400000
#define ROUNDS 60
#define EXPECT 2702659170u
#endif

static char text[SIZE + 1];

static int is_alpha(int c) {
  return (c >= 'a' && c <= 'z') || (c >= 'A' && c <= 'Z');
}

static int to_upper(int c) {
  return c >= 'a' && c <= 'z' ? c - 'a' + 'A' : c;
}

static int to_lower(int c) {
  return c >= 'A' && c <= 'Z' ? c - 'A' + 'a' : c;
}

static unsigned long long length(const char *s) {
  const char *p = s;
  while (*p) p++;
  return (unsigned long long)(p - s);
}

static unsigned scan(const char *s, unsigned *longest) {
  unsigned words = 0;
  unsigned hash_sum = 0;
  unsigned best = 0;
  const unsigned char *p = (const unsigned char *)s;
  while (*p) {
    while (*p && !is_alpha(*p)) p++;
    if (!*p) break;
    unsigned hash = 2166136261u;
    unsigned len = 0;
    while (is_alpha(*p)) {
      hash = (hash ^ (unsigned)to_lower(*p)) * 16777619u;
      p++;
      len++;
    }
    if (len > best) best = len;
    hash_sum += hash;
    words++;
  }
  *longest = best;
  return hash_sum ^ words;
}

static void flip_case(char *s, int upper) {
  for (; *s; s++) *s = (char)(upper ? to_upper(*s) : to_lower(*s));
}

int main(void) {
  unsigned x = 987654321u;
  for (int i = 0; i < SIZE; i++) {
    x = x * 1103515245u + 12345u;
    unsigned r = (x >> 16) & 31u;
    text[i] = r < 5 ? ' ' : r < 6 ? ',' : (char)('a' + (r * 7u + (x >> 24)) % 26u);
  }
  text[SIZE] = 0;
  unsigned checksum = 0;
  for (int round = 0; round < ROUNDS; round++) {
    unsigned longest = 0;
    checksum = checksum * 7u + scan(text, &longest);
    checksum += longest;
    flip_case(text, round & 1);
    checksum += (unsigned)length(text + round);
  }
  printf("strings: checksum %u\n", checksum);
  return checksum == EXPECT ? 0 : 1;
}
