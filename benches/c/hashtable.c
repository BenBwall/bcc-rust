/* An open-addressing hash table of unsigned keys with linear probing:
 * inserts pseudo-random keys, then looks up a mix of present and absent
 * keys. Pointer-free random memory access and a multiplicative hash. */
int printf(const char *, ...);

#ifdef BENCH_REDUCED
#define SLOTS_LOG2 12
#define OPERATIONS 2000
#define LOOKUP_ROUNDS 4
#define EXPECT 2689964560u
#else
#define SLOTS_LOG2 21
#define OPERATIONS 1200000
#define LOOKUP_ROUNDS 10
#define EXPECT 2460271308u
#endif

#define SLOTS (1u << SLOTS_LOG2)

static unsigned keys[SLOTS];
static unsigned values[SLOTS];
static unsigned char used[SLOTS];

static unsigned slot_of(unsigned key) {
  return (key * 2654435761u) >> (32 - SLOTS_LOG2);
}

static void insert(unsigned key, unsigned value) {
  unsigned slot = slot_of(key);
  while (used[slot] && keys[slot] != key) slot = (slot + 1) & (SLOTS - 1);
  used[slot] = 1;
  keys[slot] = key;
  values[slot] = value;
}

static int lookup(unsigned key, unsigned *value) {
  unsigned slot = slot_of(key);
  while (used[slot]) {
    if (keys[slot] == key) {
      *value = values[slot];
      return 1;
    }
    slot = (slot + 1) & (SLOTS - 1);
  }
  return 0;
}

int main(void) {
  unsigned x = 1u;
  for (unsigned i = 0; i < OPERATIONS; i++) {
    x = x * 1664525u + 1013904223u;
    insert(x, i);
  }
  unsigned found = 0;
  unsigned sum = 0;
  x = 1u;
  for (unsigned round = 0; round < LOOKUP_ROUNDS; round++) {
    for (unsigned i = 0; i < OPERATIONS; i++) {
      x = x * 1664525u + 1013904223u;
      /* Even rounds probe stored keys, odd rounds perturbed ones. */
      unsigned key = (round & 1) ? x ^ 0x5bd1e995u : x;
      unsigned value;
      if (lookup(key, &value)) {
        found++;
        sum += value;
      }
    }
    x = 1u;
  }
  unsigned checksum = found * 16777619u + sum;
  printf("hashtable: found %u sum %u checksum %u\n", found, sum, checksum);
  return checksum == EXPECT ? 0 : 1;
}
