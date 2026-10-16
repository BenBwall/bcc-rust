/* CRC-32 (the reflected 0xEDB88320 polynomial): builds the 256-entry table
 * with shifts and xors, then checksums a pseudo-random buffer several times
 * a byte at a time. A tight loop-carried dependence through a table load. */
int printf(const char *, ...);

#ifdef BENCH_REDUCED
#define SIZE 60000
#define ROUNDS 2
#define EXPECT 2466397030u
#else
#define SIZE 1000000
#define ROUNDS 180
#define EXPECT 3833129433u
#endif

static unsigned table[256];
static unsigned char buffer[SIZE];

static void build_table(void) {
  for (unsigned n = 0; n < 256; n++) {
    unsigned c = n;
    for (int k = 0; k < 8; k++) c = (c & 1u) ? 0xEDB88320u ^ (c >> 1) : c >> 1;
    table[n] = c;
  }
}

static unsigned crc32(unsigned crc, const unsigned char *data, unsigned length) {
  crc = ~crc;
  for (unsigned i = 0; i < length; i++) crc = table[(crc ^ data[i]) & 0xffu] ^ (crc >> 8);
  return ~crc;
}

int main(void) {
  build_table();
  unsigned x = 99u;
  for (unsigned i = 0; i < SIZE; i++) {
    x = x * 1664525u + 1013904223u;
    buffer[i] = (unsigned char)(x >> 24);
  }
  unsigned crc = 0;
  for (int round = 0; round < ROUNDS; round++) {
    crc = crc32(crc, buffer, SIZE);
    buffer[round] = (unsigned char)crc;
  }
  printf("crc32: crc %u table %u\n", crc, table[255]);
  return crc == EXPECT ? 0 : 1;
}
