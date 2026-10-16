/* Deterministic zlib workload: round trips of fixed data at every level and strategy.
 *
 * Prints sizes and checksums only, so the output is identical for every
 * correct build of zlib. Used by scripts/run_program_corpus.py.
 */
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include "zlib.h"

/* The interpreter runs far fewer steps than native code: zlib_roundtrip_small.c shrinks the data. */
#ifdef ROUNDTRIP_SMALL
#define SIZE_SMALL 300
#define SIZE_LARGE 1200
#else
#define SIZE_SMALL 6000
#define SIZE_LARGE 30000
#endif

static unsigned long long state = 88172645463325252ULL;

static unsigned next_random(void)
{
    state ^= state << 13;
    state ^= state >> 7;
    state ^= state << 17;
    return (unsigned)(state >> 32);
}

static unsigned char *make_data(int kind, size_t size)
{
    static const char *words[] = {"the ", "quick ", "brown ", "fox ", "jumps ", "over ", "lazy ",
                                  "dog ", "compression ", "deflate ", "inflate ", "window ", "\n"};
    unsigned char *data = malloc(size ? size : 1);
    size_t i = 0;
    if (!data) exit(2);
    switch (kind) {
    case 0: /* English-like text */
        while (i < size) {
            const char *w = words[next_random() % (sizeof words / sizeof *words)];
            while (*w && i < size) data[i++] = (unsigned char)*w++;
        }
        break;
    case 1: /* incompressible */
        for (; i < size; i++) data[i] = (unsigned char)next_random();
        break;
    case 2: /* long runs */
        while (i < size) {
            unsigned char v = (unsigned char)next_random();
            size_t n = 1 + next_random() % 300;
            while (n-- && i < size) data[i++] = v;
        }
        break;
    case 3: /* small alphabet with repeats at many distances */
        for (; i < size; i++)
            data[i] = (i > 40 && (next_random() & 3) == 0) ? data[i - 1 - next_random() % 40]
                                                           : (unsigned char)('a' + next_random() % 5);
        break;
    default: /* binary counters */
        for (; i < size; i++) data[i] = (unsigned char)((i * 7 + (i >> 8)) & 0xFF);
        break;
    }
    return data;
}

static int roundtrip(const unsigned char *data, size_t size, int level, int strategy, int window_bits,
                     int mem_level)
{
    z_stream strm;
    uLong bound = deflateBound(NULL, (uLong)size) + 64;
    unsigned char *packed = malloc(bound), *plain = malloc(size ? size : 1);
    uLong packed_size, crc;
    int ret;
    if (!packed || !plain) exit(2);
    memset(&strm, 0, sizeof strm);
    ret = deflateInit2(&strm, level, Z_DEFLATED, window_bits, mem_level, strategy);
    if (ret != Z_OK) { printf("deflateInit2 failed: %d\n", ret); return 1; }
    strm.next_in = (Bytef *)data;
    strm.avail_in = (uInt)size;
    strm.next_out = packed;
    strm.avail_out = (uInt)bound;
    ret = deflate(&strm, Z_FINISH);
    packed_size = strm.total_out;
    deflateEnd(&strm);
    if (ret != Z_STREAM_END) { printf("deflate failed: %d\n", ret); return 1; }
    crc = crc32(0L, packed, (uInt)packed_size);

    memset(&strm, 0, sizeof strm);
    ret = inflateInit2(&strm, window_bits);
    if (ret != Z_OK) { printf("inflateInit2 failed: %d\n", ret); return 1; }
    strm.next_in = packed;
    strm.avail_in = (uInt)packed_size;
    strm.next_out = plain;
    strm.avail_out = (uInt)size;
    ret = inflate(&strm, Z_FINISH);
    inflateEnd(&strm);
    printf("level %d strategy %d wbits %d mem %d: %lu -> %lu crc %08lx %s adler %08lx\n", level,
           strategy, window_bits, mem_level, (unsigned long)size, packed_size, crc,
           (ret == Z_STREAM_END && memcmp(plain, data, size) == 0) ? "ok" : "MISMATCH",
           adler32(1L, data, (uInt)size));
    free(packed);
    free(plain);
    return 0;
}

/* Streaming: small chunks in and out, with flushes, then one byte at a time. */
static int streaming(const unsigned char *data, size_t size)
{
    z_stream strm;
    unsigned char chunk[97], *packed = malloc(size + size / 100 + 1024), *plain = malloc(size);
    size_t in_off = 0, out_len = 0, i;
    int ret;
    if (!packed || !plain) exit(2);
    memset(&strm, 0, sizeof strm);
    deflateInit(&strm, 6);
    while (in_off < size) {
        size_t n = size - in_off < 211 ? size - in_off : 211;
        int flush = (in_off / 211) % 5 == 4 ? Z_SYNC_FLUSH : Z_NO_FLUSH;
        strm.next_in = (Bytef *)data + in_off;
        strm.avail_in = (uInt)n;
        in_off += n;
        do {
            strm.next_out = chunk;
            strm.avail_out = sizeof chunk;
            deflate(&strm, flush);
            memcpy(packed + out_len, chunk, sizeof chunk - strm.avail_out);
            out_len += sizeof chunk - strm.avail_out;
        } while (strm.avail_out == 0);
    }
    do {
        strm.next_out = chunk;
        strm.avail_out = sizeof chunk;
        ret = deflate(&strm, Z_FINISH);
        memcpy(packed + out_len, chunk, sizeof chunk - strm.avail_out);
        out_len += sizeof chunk - strm.avail_out;
    } while (ret != Z_STREAM_END);
    deflateEnd(&strm);

    memset(&strm, 0, sizeof strm);
    inflateInit(&strm);
    strm.next_out = plain;
    strm.avail_out = (uInt)size;
    ret = Z_OK;
    for (i = 0; i < out_len && ret == Z_OK; i++) {
        strm.next_in = packed + i;
        strm.avail_in = 1;
        ret = inflate(&strm, Z_NO_FLUSH);
    }
    inflateEnd(&strm);
    printf("streaming: %lu -> %lu bytes, ret %d, %s\n", (unsigned long)size, (unsigned long)out_len, ret,
           memcmp(plain, data, size) == 0 ? "ok" : "MISMATCH");
    free(packed);
    free(plain);
    return 0;
}

int main(void)
{
    static const size_t sizes[] = {0, 1, 17, 1000, SIZE_LARGE};
    static const int strategies[] = {Z_DEFAULT_STRATEGY, Z_FILTERED, Z_HUFFMAN_ONLY, Z_RLE, Z_FIXED};
    int kind, level, s;
    size_t k;
    uLong total = 0;
    printf("zlib %s\n", zlibVersion());
    for (kind = 0; kind < 5; kind++) {
        size_t size = kind == 1 ? SIZE_SMALL : SIZE_LARGE;
        unsigned char *data = make_data(kind, size);
        printf("== data %d (%lu bytes) crc %08lx adler %08lx\n", kind, (unsigned long)size,
               crc32(0L, data, (uInt)size), adler32(1L, data, (uInt)size));
        for (level = 0; level <= 9; level++) roundtrip(data, size, level, Z_DEFAULT_STRATEGY, 15, 8);
        for (s = 1; s < 5; s++) roundtrip(data, size, 6, strategies[s], 15, 8);
        roundtrip(data, size, 9, Z_DEFAULT_STRATEGY, 9, 1);
        roundtrip(data, size, 5, Z_DEFAULT_STRATEGY, -15, 9);
        roundtrip(data, size, 6, Z_DEFAULT_STRATEGY, 31, 8);
        if (kind == 0 || kind == 3) streaming(data, size);
        free(data);
    }
    for (k = 0; k < sizeof sizes / sizeof *sizes; k++) {
        unsigned char *data = make_data(0, sizes[k]);
        uLongf packed_len = compressBound((uLong)sizes[k]), plain_len = (uLongf)sizes[k];
        unsigned char *packed = malloc(packed_len), *plain = malloc(sizes[k] + 1);
        int a = compress2(packed, &packed_len, data, (uLong)sizes[k], 6);
        int b = uncompress(plain, &plain_len, packed, packed_len);
        printf("compress2 size %lu -> %lu (%d %d) %s\n", (unsigned long)sizes[k], (unsigned long)packed_len,
               a, b, plain_len == sizes[k] && memcmp(plain, data, sizes[k]) == 0 ? "ok" : "MISMATCH");
        total += packed_len;
        free(data); free(packed); free(plain);
    }
    {
        unsigned char *data = make_data(0, 1000);
        unsigned char bad[64];
        uLongf out_len = 1000;
        unsigned char out[1000];
        memset(bad, 0x55, sizeof bad);
        printf("corrupt input -> %d, truncated -> %d\n",
               uncompress(out, &out_len, bad, sizeof bad), uncompress(out, &out_len, data, 3));
        free(data);
    }
    printf("crc32_combine %08lx adler32_combine %08lx total %lu\n",
           crc32_combine(0x12345678UL, 0x9abcdef0UL, 1000L), adler32_combine(0x12345678UL, 0x9abcdef0UL, 1000L), total);
    return 0;
}
