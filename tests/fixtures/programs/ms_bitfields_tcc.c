/* The bits each field occupies, and the sizes and alignments, of records
   that TCC's own Microsoft layout places, which tests/integration/
   ms_bitfields.rs compares with what TCC 0.9.28 prints under -mms-bitfields.
   TCC lays a bit-field as wide as its type out as an ordinary member, and
   a bit-field that starts no run stays at the last member's offset, so
   some fields overlap others or lie past the record: each record sits in a
   larger buffer. No header is included, as glibc's hide attributes from
   TCC. */
int printf(const char *, ...);
void *memset(void *, int, unsigned long);

/* Prints " name@first+count": the first bit set and how many are. */
static void bits(const char *name, const unsigned char *p, unsigned size) {
    int first = -1, count = 0;
    for (unsigned i = 0; i < size * 8; i++)
        if ((p[i / 8] >> (i % 8)) & 1) {
            if (first < 0)
                first = i;
            count++;
        }
    printf(" %s@%d+%d", name, first, count);
}
#define FIELD(f, value) (memset(&u, 0, sizeof u), u.v.f = value, bits(#f, u.raw, sizeof u.raw))
#define SHOW(T, ...) do { \
        union { T v; unsigned char raw[sizeof(T) + 16]; } u; \
        printf("%s %u %u", #T, (unsigned)sizeof(T), (unsigned)_Alignof(T)); \
        __VA_ARGS__; \
        printf("\n"); \
    } while (0)

/* Full-width bit-fields, then zero-width ones. */
struct R { unsigned char a:8; unsigned int :0; unsigned int b:11; };
struct F1 { char a:8; char :0; char b:3; };
struct F2 { int a:32; char :0; char b:3; };
struct F3 { long long a:64; int :0; int b:3; };
struct F4 { char a:8; int :0; int :0; int b:3; };
struct F5 { char a:8; int :0; int b:20; int c:20; };
struct F6 { char c; char a:8; int :0; int b:4; };
/* Ordinary members, then zero-width bit-fields. */
struct O1 { char x; int :0; int b:3; };
struct O2 { int x; char :0; int b:3; };
struct O3 { short x; char :0; char b:5; char c:5; };
struct O4 { char x; _Bool :0; _Bool b:1; };
/* Zero-width bit-fields of each type between and before others. */
struct Z1 { char a:3; short :0; int b:5; };
struct Z2 { char a:3; int :0; int b:5; };
struct Z3 { int a:3; long long :0; long long b:3; };
struct Z4 { short a:3; int :0; char b:3; };
struct Z5 { char :0; char c:3; };
struct Z6 { long long :0; char c:3; int d:2; };
union U1 { char a:8; int :0; int b:11; };
union U2 { char a:3; long long :0; };
/* A pack or `packed` keeps a full-width bit-field one. */
#pragma pack(push, 1)
struct P1 { unsigned char a:8; unsigned int :0; unsigned int b:11; };
#pragma pack(pop)
#pragma pack(push, 2)
struct P2 { char x; int :0; int b:3; int c:30; };
#pragma pack(pop)
struct __attribute__((packed)) P3 { unsigned char a:8; unsigned int :0; unsigned int b:11; };
struct P4 { unsigned char a:8 __attribute__((packed)); unsigned int :0; unsigned int b:11; };

int main(void) {
    SHOW(struct R, FIELD(a, -1), FIELD(b, -1));
    SHOW(struct F1, FIELD(a, -1), FIELD(b, -1));
    SHOW(struct F2, FIELD(a, -1), FIELD(b, -1));
    SHOW(struct F3, FIELD(a, -1), FIELD(b, -1));
    SHOW(struct F4, FIELD(a, -1), FIELD(b, -1));
    SHOW(struct F5, FIELD(a, -1), FIELD(b, -1), FIELD(c, -1));
    SHOW(struct F6, FIELD(c, -1), FIELD(a, -1), FIELD(b, -1));
    SHOW(struct O1, FIELD(x, -1), FIELD(b, -1));
    SHOW(struct O2, FIELD(x, -1), FIELD(b, -1));
    SHOW(struct O3, FIELD(x, -1), FIELD(b, -1), FIELD(c, -1));
    SHOW(struct O4, FIELD(x, -1), FIELD(b, 1));
    SHOW(struct Z1, FIELD(a, -1), FIELD(b, -1));
    SHOW(struct Z2, FIELD(a, -1), FIELD(b, -1));
    SHOW(struct Z3, FIELD(a, -1), FIELD(b, -1));
    SHOW(struct Z4, FIELD(a, -1), FIELD(b, -1));
    SHOW(struct Z5, FIELD(c, -1));
    SHOW(struct Z6, FIELD(c, -1), FIELD(d, -1));
    SHOW(union U1, FIELD(a, -1), FIELD(b, -1));
    SHOW(union U2, FIELD(a, -1));
    SHOW(struct P1, FIELD(a, -1), FIELD(b, -1));
    SHOW(struct P2, FIELD(x, -1), FIELD(b, -1), FIELD(c, -1));
    SHOW(struct P3, FIELD(a, -1), FIELD(b, -1));
    SHOW(struct P4, FIELD(a, -1), FIELD(b, -1));
    return 0;
}
