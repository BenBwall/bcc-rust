int putchar(int);
static void ps(const char *s) { while (*s) putchar(*s++); }
static void pu(unsigned long long v) { char b[24]; int n = 0; do b[n++] = (char)('0' + v % 10); while (v /= 10); while (n) putchar(b[--n]); }
static void pi(long long v) { if (v < 0) { putchar('-'); pu(0ull - (unsigned long long)v); } else pu((unsigned long long)v); }
static void img(const void *p, unsigned long long n) { const unsigned char *b = (const unsigned char *)p; putchar(':'); while (n--) { putchar("0123456789abcdef"[*b >> 4]); putchar("0123456789abcdef"[*b & 15]); b++; } }
static void fill(void *p, int v, unsigned long long n) { unsigned char *b = (unsigned char *)p; while (n--) *b++ = (unsigned char)v; }
#pragma pack(push, 2)
struct T0 { unsigned f0[3]; signed char : 0; unsigned long long : 0; };
#pragma pack(pop)
union T1 { char f0[1]; _Bool f1 : 1; unsigned short f2 : 10; int f3 : 4; int f4 : 24; signed char f5 : 6; unsigned long long f6 : 20; } __attribute__((packed));
#pragma pack(push, 2)
struct T2 { int f0; signed char f1; unsigned short f2; unsigned f3; _Bool : 0; char f5 : 4; };
#pragma pack(pop)
#pragma pack(push, 2)
struct T3 { union T1 f0; int f1; unsigned char f2 : 8; };
#pragma pack(pop)
struct T4 { unsigned char f0; char f1 : 7; char : 8; char f3 : 6; unsigned char f4; signed char f5; };
struct T5 { struct T0 f0; short f1; struct T0 f2; unsigned short f3 : 12; short f4; };
struct T6 { short f0 : 1; _Bool f1 : 1; signed char f2 : 4; char f3 : 2; } __attribute__((packed));
struct T7 { int : 26; unsigned char : 0; unsigned char f2 : 3; unsigned char : 0; char f4 __attribute__((aligned(2))); struct T6 f5; short f6 : 8; };
#pragma pack(push, 2)
struct T8 { _Bool f0 : 1; unsigned long long f1 : 24; long long f2[1]; int f3; unsigned long long f4 : 6; long long f5 : 26; unsigned long long f6; unsigned short f7; };
#pragma pack(pop)
struct T9 { short f0 : 10; int f1 : 30; };
#pragma pack(push, 1)
struct T10 { unsigned f0; signed char f1[3]; long long : 0; };
#pragma pack(pop)
#pragma pack(push, 2)
struct T11 { signed char f0; unsigned short f1 : 6; };
#pragma pack(pop)
struct T12 { int : 0; unsigned char : 0; char f2 __attribute__((aligned(2))); signed char : 0; char f4 : 5; struct T2 f5; short f6; long long : 45; };
#pragma pack(push, 8)
struct T13 { unsigned long long f0 : 14; unsigned long long f1 : 30; unsigned f2 __attribute__((packed)); unsigned short : 0; };
#pragma pack(pop)
#pragma pack(push, 8)
struct T14 { _Bool f0 : 1; unsigned short f1 : 11; unsigned char f2 : 8; signed char : 0; unsigned f4 : 20; };
#pragma pack(pop)
struct T15 { int f0; unsigned char f1 : 7; _Bool f2 : 1; unsigned f3; short : 0; signed char f5; signed char f6; };
struct T16 { long long f0[3]; unsigned short f1; unsigned char f2 : 5; unsigned short f3[1]; long long f4 : 60; unsigned short f5 __attribute__((aligned(8))); signed char f6; short f7 __attribute__((aligned(1))); };
#pragma pack(push, 1)
struct T17 { unsigned char f0; int f1 : 32; long long f2 : 54; int f3[1]; struct T11 f4; signed char f5 : 2; };
#pragma pack(pop)
struct T18 { unsigned char f0 : 6; unsigned f1; char f2; unsigned short f3 : 5; unsigned short f4 : 16; };
struct T19 { unsigned long long f0 : 58; unsigned long long : 34; unsigned : 14; unsigned short f3 : 7; unsigned f4 __attribute__((packed)); char : 8; unsigned long long f6; unsigned char f7 : 2; };
#pragma pack(push, 4)
union T20 { short : 0; unsigned short f1; signed char f2 : 4; };
#pragma pack(pop)
#pragma pack(push, 2)
struct T21 { short f0 : 8 __attribute__((aligned(8))); char f1 : 7; short : 14; unsigned short f3; struct T14 f4; short f5 : 4; };
#pragma pack(pop)
#pragma pack(push, 8)
struct T22 { unsigned : 0; _Bool : 0; struct T14 f2; };
#pragma pack(pop)
#pragma pack(push, 1)
struct T23 { unsigned char f0 : 8 __attribute__((aligned(8))); long long : 0; };
#pragma pack(pop)
struct T24 { struct T11 f0; unsigned short f1 __attribute__((aligned(8))); };
struct T25 { int f0 : 2; char f1 : 1; int f2 : 3; unsigned short f3 : 3 __attribute__((aligned(2))); } __attribute__((packed));
struct T26 { long long f0 : 2; _Bool f1 : 1; struct T2 f2; char f3 : 3; _Bool : 0; signed char f5 : 8; short f6; } __attribute__((packed));
#pragma pack(push, 4)
struct T27 { int f0 : 29; _Bool f1 : 1; signed char f2 __attribute__((aligned(8))); short f3 __attribute__((aligned(4))); short f4 : 7; };
#pragma pack(pop)
#pragma pack(push, 1)
struct T28 { signed char f0 __attribute__((packed)); };
#pragma pack(pop)
#pragma pack(push, 8)
struct T29 { char f0 : 4; unsigned char f1[1]; long long : 0; signed char f3; unsigned char : 5; };
#pragma pack(pop)
#pragma pack(push, 2)
struct T30 { char f0; unsigned : 0; unsigned char f2 : 7; char : 0; unsigned f4 : 24; unsigned long long : 0; unsigned : 13; };
#pragma pack(pop)
struct T31 { signed char f0; short f1; _Bool f2 : 1; short f3; char f4; unsigned short f5; int f6 : 2 __attribute__((aligned(8))); } __attribute__((packed));
#pragma pack(push, 8)
struct T32 { long long : 0; int : 0; long long f2 : 6; };
#pragma pack(pop)
#pragma pack(push, 1)
union T33 { unsigned char : 0; _Bool f1 : 1; short f2 : 9; long long f3 : 62; char f4 : 5; unsigned short f5 : 12; unsigned short f6; };
#pragma pack(pop)
#pragma pack(push, 8)
struct T34 { int : 0; short f1 : 2; int f2; };
#pragma pack(pop)
struct T35 { unsigned char f0 : 1 __attribute__((aligned(1))); };
struct T36 { _Bool f0 : 1; struct T9 f1; long long : 35; unsigned long long f3; struct T0 f4; char : 0; unsigned char : 0; } __attribute__((packed));
#pragma pack(push, 8)
struct T37 { unsigned char f0 : 7; char f1; unsigned f2; long long f3 : 34 __attribute__((packed)); };
#pragma pack(pop)
#pragma pack(push, 1)
struct T38 { unsigned short f0; };
#pragma pack(pop)
struct T39 { signed char f0[2]; struct T10 f1; int f2 : 16; unsigned long long f3 : 1; } __attribute__((packed));
#pragma pack(push, 8)
struct T40 { _Bool f0 : 1 __attribute__((aligned(1))); int f1 __attribute__((aligned(2))); };
#pragma pack(pop)
union T41 { char f0 : 6; signed char f1; };
union T42 { signed char f0; };
#pragma pack(push, 2)
struct T43 { int f0 : 23; int f1; char f2[3]; signed char f3 : 4; unsigned char : 5; };
#pragma pack(pop)
#pragma pack(push, 8)
struct T44 { long long f0 : 34; short : 0; signed char f2 : 2; };
#pragma pack(pop)
struct T45 { int f0 : 11; };
#pragma pack(push, 8)
struct T46 { int f0 : 29; };
#pragma pack(pop)
struct T47 { int f0 : 16; _Bool f1 : 1; unsigned char f2 : 5; };
#pragma pack(push, 4)
struct T48 { unsigned short f0 : 4; unsigned f1; unsigned short : 0; _Bool f3 : 1; short f4; };
#pragma pack(pop)
#pragma pack(push, 8)
struct T49 { long long f0 : 13; unsigned char f1 : 8 __attribute__((aligned(8))); };
#pragma pack(pop)
union T50 { signed char f0 : 3; signed char f1; char f2; long long f3 : 51; char f4 : 1 __attribute__((aligned(8))); _Bool f5 : 1; unsigned f6 : 5; unsigned char f7 : 7; } __attribute__((packed));
struct T51 { unsigned short f0; long long f1 : 36; _Bool f2 : 1; _Bool f3 : 1; };
struct T52 { _Bool : 0; struct T28 f1; struct T25 f2; unsigned long long f3 : 12; short f4 : 3; };
#pragma pack(push, 8)
struct T53 { long long : 0; long long f1; _Bool f2 : 1; };
#pragma pack(pop)
#pragma pack(push, 1)
struct T54 { unsigned f0; char : 0; int f2[2]; long long f3[1]; unsigned f4; long long f5; int f6 : 19; };
#pragma pack(pop)
#pragma pack(push, 8)
struct T55 { long long f0; _Bool f1 : 1; unsigned long long f2 : 14 __attribute__((aligned(2))); _Bool f3 : 1; };
#pragma pack(pop)
struct T56 { short : 6; signed char f1; } __attribute__((packed));
#pragma pack(push, 1)
struct T57 { unsigned char f0 : 7; _Bool f1 : 1; long long f2 : 20; long long f3 : 2 __attribute__((packed)); short f4[1]; };
#pragma pack(pop)
#pragma pack(push, 4)
struct T58 { char f0 : 3; long long : 33; char f2 : 2; };
#pragma pack(pop)
#pragma pack(push, 2)
struct T59 { long long f0[1]; struct T51 f1; unsigned f2 : 29; unsigned short : 11; unsigned char f4; unsigned : 15; unsigned short f6 : 2; unsigned short f7; };
#pragma pack(pop)
static void t0(void) {
    struct T0 s; ps("T0 "); pu(sizeof s); putchar(' '); pu(_Alignof(struct T0));
    fill(&s, 0, sizeof s); s.f0[0] = -1; ps(" f0"); img(&s, sizeof s);
    putchar('\n');
}
static void t1(void) {
    union T1 s; ps("T1 "); pu(sizeof s); putchar(' '); pu(_Alignof(union T1));
    fill(&s, 0, sizeof s); s.f0[0] = -1; ps(" f0"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f1 = -1; ps(" f1"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f2 = -1; ps(" f2"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f3 = -1; ps(" f3"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f4 = -1; ps(" f4"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f5 = -1; ps(" f5"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f6 = -1; ps(" f6"); img(&s, sizeof s);
    fill(&s, 0x5a, sizeof s); s.f1 = 0LL; ps(" f1="); pi((long long)s.f1); img(&s, sizeof s);
    fill(&s, 0x5a, sizeof s); s.f2 = 384LL; ps(" f2="); pi((long long)s.f2); img(&s, sizeof s);
    fill(&s, 0x5a, sizeof s); s.f3 = 5LL; ps(" f3="); pi((long long)s.f3); img(&s, sizeof s);
    fill(&s, 0x5a, sizeof s); s.f4 = -4293348LL; ps(" f4="); pi((long long)s.f4); img(&s, sizeof s);
    fill(&s, 0x5a, sizeof s); s.f5 = -24LL; ps(" f5="); pi((long long)s.f5); img(&s, sizeof s);
    fill(&s, 0x5a, sizeof s); s.f6 = 162781LL; ps(" f6="); pi((long long)s.f6); img(&s, sizeof s);
    putchar('\n');
}
static void t2(void) {
    struct T2 s; ps("T2 "); pu(sizeof s); putchar(' '); pu(_Alignof(struct T2));
    fill(&s, 0, sizeof s); s.f0 = -1; ps(" f0"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f1 = -1; ps(" f1"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f2 = -1; ps(" f2"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f3 = -1; ps(" f3"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f5 = -1; ps(" f5"); img(&s, sizeof s);
    fill(&s, 0x5a, sizeof s); s.f5 = -5LL; ps(" f5="); pi((long long)s.f5); img(&s, sizeof s);
    putchar('\n');
}
static void t3(void) {
    struct T3 s; ps("T3 "); pu(sizeof s); putchar(' '); pu(_Alignof(struct T3));
    fill(&s, 0, sizeof s); fill(&s.f0, 0xff, sizeof s.f0); ps(" f0"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f1 = -1; ps(" f1"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f2 = -1; ps(" f2"); img(&s, sizeof s);
    fill(&s, 0x5a, sizeof s); s.f2 = 74LL; ps(" f2="); pi((long long)s.f2); img(&s, sizeof s);
    putchar('\n');
}
static void t4(void) {
    struct T4 s; ps("T4 "); pu(sizeof s); putchar(' '); pu(_Alignof(struct T4));
    fill(&s, 0, sizeof s); s.f0 = -1; ps(" f0"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f1 = -1; ps(" f1"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f3 = -1; ps(" f3"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f4 = -1; ps(" f4"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f5 = -1; ps(" f5"); img(&s, sizeof s);
    fill(&s, 0x5a, sizeof s); s.f1 = -12LL; ps(" f1="); pi((long long)s.f1); img(&s, sizeof s);
    fill(&s, 0x5a, sizeof s); s.f3 = -7LL; ps(" f3="); pi((long long)s.f3); img(&s, sizeof s);
    putchar('\n');
}
static void t5(void) {
    struct T5 s; ps("T5 "); pu(sizeof s); putchar(' '); pu(_Alignof(struct T5));
    fill(&s, 0, sizeof s); fill(&s.f0, 0xff, sizeof s.f0); ps(" f0"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f1 = -1; ps(" f1"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); fill(&s.f2, 0xff, sizeof s.f2); ps(" f2"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f3 = -1; ps(" f3"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f4 = -1; ps(" f4"); img(&s, sizeof s);
    fill(&s, 0x5a, sizeof s); s.f3 = 2700LL; ps(" f3="); pi((long long)s.f3); img(&s, sizeof s);
    putchar('\n');
}
static void t6(void) {
    struct T6 s; ps("T6 "); pu(sizeof s); putchar(' '); pu(_Alignof(struct T6));
    fill(&s, 0, sizeof s); s.f0 = -1; ps(" f0"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f1 = -1; ps(" f1"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f2 = -1; ps(" f2"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f3 = -1; ps(" f3"); img(&s, sizeof s);
    fill(&s, 0x5a, sizeof s); s.f0 = 0LL; ps(" f0="); pi((long long)s.f0); img(&s, sizeof s);
    fill(&s, 0x5a, sizeof s); s.f1 = 1LL; ps(" f1="); pi((long long)s.f1); img(&s, sizeof s);
    fill(&s, 0x5a, sizeof s); s.f2 = 2LL; ps(" f2="); pi((long long)s.f2); img(&s, sizeof s);
    fill(&s, 0x5a, sizeof s); s.f3 = 1LL; ps(" f3="); pi((long long)s.f3); img(&s, sizeof s);
    putchar('\n');
}
static void t7(void) {
    struct T7 s; ps("T7 "); pu(sizeof s); putchar(' '); pu(_Alignof(struct T7));
    fill(&s, 0, sizeof s); s.f2 = -1; ps(" f2"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f4 = -1; ps(" f4"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); fill(&s.f5, 0xff, sizeof s.f5); ps(" f5"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f6 = -1; ps(" f6"); img(&s, sizeof s);
    fill(&s, 0x5a, sizeof s); s.f2 = 5LL; ps(" f2="); pi((long long)s.f2); img(&s, sizeof s);
    fill(&s, 0x5a, sizeof s); s.f6 = 86LL; ps(" f6="); pi((long long)s.f6); img(&s, sizeof s);
    putchar('\n');
}
static void t8(void) {
    struct T8 s; ps("T8 "); pu(sizeof s); putchar(' '); pu(_Alignof(struct T8));
    fill(&s, 0, sizeof s); s.f0 = -1; ps(" f0"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f1 = -1; ps(" f1"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f2[0] = -1; ps(" f2"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f3 = -1; ps(" f3"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f4 = -1; ps(" f4"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f5 = -1; ps(" f5"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f6 = -1; ps(" f6"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f7 = -1; ps(" f7"); img(&s, sizeof s);
    fill(&s, 0x5a, sizeof s); s.f0 = 0LL; ps(" f0="); pi((long long)s.f0); img(&s, sizeof s);
    fill(&s, 0x5a, sizeof s); s.f1 = 4749930LL; ps(" f1="); pi((long long)s.f1); img(&s, sizeof s);
    fill(&s, 0x5a, sizeof s); s.f4 = 8LL; ps(" f4="); pi((long long)s.f4); img(&s, sizeof s);
    fill(&s, 0x5a, sizeof s); s.f5 = -1473957LL; ps(" f5="); pi((long long)s.f5); img(&s, sizeof s);
    putchar('\n');
}
static void t9(void) {
    struct T9 s; ps("T9 "); pu(sizeof s); putchar(' '); pu(_Alignof(struct T9));
    fill(&s, 0, sizeof s); s.f0 = -1; ps(" f0"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f1 = -1; ps(" f1"); img(&s, sizeof s);
    fill(&s, 0x5a, sizeof s); s.f0 = 222LL; ps(" f0="); pi((long long)s.f0); img(&s, sizeof s);
    fill(&s, 0x5a, sizeof s); s.f1 = -13678633LL; ps(" f1="); pi((long long)s.f1); img(&s, sizeof s);
    putchar('\n');
}
static void t10(void) {
    struct T10 s; ps("T10 "); pu(sizeof s); putchar(' '); pu(_Alignof(struct T10));
    fill(&s, 0, sizeof s); s.f0 = -1; ps(" f0"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f1[0] = -1; ps(" f1"); img(&s, sizeof s);
    putchar('\n');
}
static void t11(void) {
    struct T11 s; ps("T11 "); pu(sizeof s); putchar(' '); pu(_Alignof(struct T11));
    fill(&s, 0, sizeof s); s.f0 = -1; ps(" f0"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f1 = -1; ps(" f1"); img(&s, sizeof s);
    fill(&s, 0x5a, sizeof s); s.f1 = 33LL; ps(" f1="); pi((long long)s.f1); img(&s, sizeof s);
    putchar('\n');
}
static void t12(void) {
    struct T12 s; ps("T12 "); pu(sizeof s); putchar(' '); pu(_Alignof(struct T12));
    fill(&s, 0, sizeof s); s.f2 = -1; ps(" f2"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f4 = -1; ps(" f4"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); fill(&s.f5, 0xff, sizeof s.f5); ps(" f5"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f6 = -1; ps(" f6"); img(&s, sizeof s);
    fill(&s, 0x5a, sizeof s); s.f4 = 15LL; ps(" f4="); pi((long long)s.f4); img(&s, sizeof s);
    putchar('\n');
}
static void t13(void) {
    struct T13 s; ps("T13 "); pu(sizeof s); putchar(' '); pu(_Alignof(struct T13));
    fill(&s, 0, sizeof s); s.f0 = -1; ps(" f0"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f1 = -1; ps(" f1"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f2 = -1; ps(" f2"); img(&s, sizeof s);
    fill(&s, 0x5a, sizeof s); s.f0 = 12880LL; ps(" f0="); pi((long long)s.f0); img(&s, sizeof s);
    fill(&s, 0x5a, sizeof s); s.f1 = 300043863LL; ps(" f1="); pi((long long)s.f1); img(&s, sizeof s);
    putchar('\n');
}
static void t14(void) {
    struct T14 s; ps("T14 "); pu(sizeof s); putchar(' '); pu(_Alignof(struct T14));
    fill(&s, 0, sizeof s); s.f0 = -1; ps(" f0"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f1 = -1; ps(" f1"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f2 = -1; ps(" f2"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f4 = -1; ps(" f4"); img(&s, sizeof s);
    fill(&s, 0x5a, sizeof s); s.f0 = 0LL; ps(" f0="); pi((long long)s.f0); img(&s, sizeof s);
    fill(&s, 0x5a, sizeof s); s.f1 = 1460LL; ps(" f1="); pi((long long)s.f1); img(&s, sizeof s);
    fill(&s, 0x5a, sizeof s); s.f2 = 2LL; ps(" f2="); pi((long long)s.f2); img(&s, sizeof s);
    fill(&s, 0x5a, sizeof s); s.f4 = 325587LL; ps(" f4="); pi((long long)s.f4); img(&s, sizeof s);
    putchar('\n');
}
static void t15(void) {
    struct T15 s; ps("T15 "); pu(sizeof s); putchar(' '); pu(_Alignof(struct T15));
    fill(&s, 0, sizeof s); s.f0 = -1; ps(" f0"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f1 = -1; ps(" f1"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f2 = -1; ps(" f2"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f3 = -1; ps(" f3"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f5 = -1; ps(" f5"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f6 = -1; ps(" f6"); img(&s, sizeof s);
    fill(&s, 0x5a, sizeof s); s.f1 = 4LL; ps(" f1="); pi((long long)s.f1); img(&s, sizeof s);
    fill(&s, 0x5a, sizeof s); s.f2 = 0LL; ps(" f2="); pi((long long)s.f2); img(&s, sizeof s);
    putchar('\n');
}
static void t16(void) {
    struct T16 s; ps("T16 "); pu(sizeof s); putchar(' '); pu(_Alignof(struct T16));
    fill(&s, 0, sizeof s); s.f0[0] = -1; ps(" f0"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f1 = -1; ps(" f1"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f2 = -1; ps(" f2"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f3[0] = -1; ps(" f3"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f4 = -1; ps(" f4"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f5 = -1; ps(" f5"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f6 = -1; ps(" f6"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f7 = -1; ps(" f7"); img(&s, sizeof s);
    fill(&s, 0x5a, sizeof s); s.f2 = 31LL; ps(" f2="); pi((long long)s.f2); img(&s, sizeof s);
    fill(&s, 0x5a, sizeof s); s.f4 = -346741108403845313LL; ps(" f4="); pi((long long)s.f4); img(&s, sizeof s);
    putchar('\n');
}
static void t17(void) {
    struct T17 s; ps("T17 "); pu(sizeof s); putchar(' '); pu(_Alignof(struct T17));
    fill(&s, 0, sizeof s); s.f0 = -1; ps(" f0"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f1 = -1; ps(" f1"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f2 = -1; ps(" f2"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f3[0] = -1; ps(" f3"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); fill(&s.f4, 0xff, sizeof s.f4); ps(" f4"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f5 = -1; ps(" f5"); img(&s, sizeof s);
    fill(&s, 0x5a, sizeof s); s.f1 = 779915018LL; ps(" f1="); pi((long long)s.f1); img(&s, sizeof s);
    fill(&s, 0x5a, sizeof s); s.f2 = -3160670720614523LL; ps(" f2="); pi((long long)s.f2); img(&s, sizeof s);
    fill(&s, 0x5a, sizeof s); s.f5 = -1LL; ps(" f5="); pi((long long)s.f5); img(&s, sizeof s);
    putchar('\n');
}
static void t18(void) {
    struct T18 s; ps("T18 "); pu(sizeof s); putchar(' '); pu(_Alignof(struct T18));
    fill(&s, 0, sizeof s); s.f0 = -1; ps(" f0"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f1 = -1; ps(" f1"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f2 = -1; ps(" f2"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f3 = -1; ps(" f3"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f4 = -1; ps(" f4"); img(&s, sizeof s);
    fill(&s, 0x5a, sizeof s); s.f0 = 3LL; ps(" f0="); pi((long long)s.f0); img(&s, sizeof s);
    fill(&s, 0x5a, sizeof s); s.f3 = 16LL; ps(" f3="); pi((long long)s.f3); img(&s, sizeof s);
    fill(&s, 0x5a, sizeof s); s.f4 = 21188LL; ps(" f4="); pi((long long)s.f4); img(&s, sizeof s);
    putchar('\n');
}
static void t19(void) {
    struct T19 s; ps("T19 "); pu(sizeof s); putchar(' '); pu(_Alignof(struct T19));
    fill(&s, 0, sizeof s); s.f0 = -1; ps(" f0"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f3 = -1; ps(" f3"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f4 = -1; ps(" f4"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f6 = -1; ps(" f6"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f7 = -1; ps(" f7"); img(&s, sizeof s);
    fill(&s, 0x5a, sizeof s); s.f0 = 197123769682678092LL; ps(" f0="); pi((long long)s.f0); img(&s, sizeof s);
    fill(&s, 0x5a, sizeof s); s.f3 = 114LL; ps(" f3="); pi((long long)s.f3); img(&s, sizeof s);
    fill(&s, 0x5a, sizeof s); s.f7 = 3LL; ps(" f7="); pi((long long)s.f7); img(&s, sizeof s);
    putchar('\n');
}
static void t20(void) {
    union T20 s; ps("T20 "); pu(sizeof s); putchar(' '); pu(_Alignof(union T20));
    fill(&s, 0, sizeof s); s.f1 = -1; ps(" f1"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f2 = -1; ps(" f2"); img(&s, sizeof s);
    fill(&s, 0x5a, sizeof s); s.f2 = -7LL; ps(" f2="); pi((long long)s.f2); img(&s, sizeof s);
    putchar('\n');
}
static void t21(void) {
    struct T21 s; ps("T21 "); pu(sizeof s); putchar(' '); pu(_Alignof(struct T21));
    fill(&s, 0, sizeof s); s.f0 = -1; ps(" f0"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f1 = -1; ps(" f1"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f3 = -1; ps(" f3"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); fill(&s.f4, 0xff, sizeof s.f4); ps(" f4"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f5 = -1; ps(" f5"); img(&s, sizeof s);
    fill(&s, 0x5a, sizeof s); s.f0 = -49LL; ps(" f0="); pi((long long)s.f0); img(&s, sizeof s);
    fill(&s, 0x5a, sizeof s); s.f1 = -16LL; ps(" f1="); pi((long long)s.f1); img(&s, sizeof s);
    fill(&s, 0x5a, sizeof s); s.f5 = -4LL; ps(" f5="); pi((long long)s.f5); img(&s, sizeof s);
    putchar('\n');
}
static void t22(void) {
    struct T22 s; ps("T22 "); pu(sizeof s); putchar(' '); pu(_Alignof(struct T22));
    fill(&s, 0, sizeof s); fill(&s.f2, 0xff, sizeof s.f2); ps(" f2"); img(&s, sizeof s);
    putchar('\n');
}
static void t23(void) {
    struct T23 s; ps("T23 "); pu(sizeof s); putchar(' '); pu(_Alignof(struct T23));
    fill(&s, 0, sizeof s); s.f0 = -1; ps(" f0"); img(&s, sizeof s);
    fill(&s, 0x5a, sizeof s); s.f0 = 169LL; ps(" f0="); pi((long long)s.f0); img(&s, sizeof s);
    putchar('\n');
}
static void t24(void) {
    struct T24 s; ps("T24 "); pu(sizeof s); putchar(' '); pu(_Alignof(struct T24));
    fill(&s, 0, sizeof s); fill(&s.f0, 0xff, sizeof s.f0); ps(" f0"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f1 = -1; ps(" f1"); img(&s, sizeof s);
    putchar('\n');
}
static void t25(void) {
    struct T25 s; ps("T25 "); pu(sizeof s); putchar(' '); pu(_Alignof(struct T25));
    fill(&s, 0, sizeof s); s.f0 = -1; ps(" f0"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f1 = -1; ps(" f1"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f2 = -1; ps(" f2"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f3 = -1; ps(" f3"); img(&s, sizeof s);
    fill(&s, 0x5a, sizeof s); s.f0 = 0LL; ps(" f0="); pi((long long)s.f0); img(&s, sizeof s);
    fill(&s, 0x5a, sizeof s); s.f1 = 0LL; ps(" f1="); pi((long long)s.f1); img(&s, sizeof s);
    fill(&s, 0x5a, sizeof s); s.f2 = -1LL; ps(" f2="); pi((long long)s.f2); img(&s, sizeof s);
    fill(&s, 0x5a, sizeof s); s.f3 = 7LL; ps(" f3="); pi((long long)s.f3); img(&s, sizeof s);
    putchar('\n');
}
static void t26(void) {
    struct T26 s; ps("T26 "); pu(sizeof s); putchar(' '); pu(_Alignof(struct T26));
    fill(&s, 0, sizeof s); s.f0 = -1; ps(" f0"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f1 = -1; ps(" f1"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); fill(&s.f2, 0xff, sizeof s.f2); ps(" f2"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f3 = -1; ps(" f3"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f5 = -1; ps(" f5"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f6 = -1; ps(" f6"); img(&s, sizeof s);
    fill(&s, 0x5a, sizeof s); s.f0 = -1LL; ps(" f0="); pi((long long)s.f0); img(&s, sizeof s);
    fill(&s, 0x5a, sizeof s); s.f1 = 0LL; ps(" f1="); pi((long long)s.f1); img(&s, sizeof s);
    fill(&s, 0x5a, sizeof s); s.f3 = -1LL; ps(" f3="); pi((long long)s.f3); img(&s, sizeof s);
    fill(&s, 0x5a, sizeof s); s.f5 = 101LL; ps(" f5="); pi((long long)s.f5); img(&s, sizeof s);
    putchar('\n');
}
static void t27(void) {
    struct T27 s; ps("T27 "); pu(sizeof s); putchar(' '); pu(_Alignof(struct T27));
    fill(&s, 0, sizeof s); s.f0 = -1; ps(" f0"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f1 = -1; ps(" f1"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f2 = -1; ps(" f2"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f3 = -1; ps(" f3"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f4 = -1; ps(" f4"); img(&s, sizeof s);
    fill(&s, 0x5a, sizeof s); s.f0 = 206248526LL; ps(" f0="); pi((long long)s.f0); img(&s, sizeof s);
    fill(&s, 0x5a, sizeof s); s.f1 = 0LL; ps(" f1="); pi((long long)s.f1); img(&s, sizeof s);
    fill(&s, 0x5a, sizeof s); s.f4 = -6LL; ps(" f4="); pi((long long)s.f4); img(&s, sizeof s);
    putchar('\n');
}
static void t28(void) {
    struct T28 s; ps("T28 "); pu(sizeof s); putchar(' '); pu(_Alignof(struct T28));
    fill(&s, 0, sizeof s); s.f0 = -1; ps(" f0"); img(&s, sizeof s);
    putchar('\n');
}
static void t29(void) {
    struct T29 s; ps("T29 "); pu(sizeof s); putchar(' '); pu(_Alignof(struct T29));
    fill(&s, 0, sizeof s); s.f0 = -1; ps(" f0"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f1[0] = -1; ps(" f1"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f3 = -1; ps(" f3"); img(&s, sizeof s);
    fill(&s, 0x5a, sizeof s); s.f0 = -2LL; ps(" f0="); pi((long long)s.f0); img(&s, sizeof s);
    putchar('\n');
}
static void t30(void) {
    struct T30 s; ps("T30 "); pu(sizeof s); putchar(' '); pu(_Alignof(struct T30));
    fill(&s, 0, sizeof s); s.f0 = -1; ps(" f0"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f2 = -1; ps(" f2"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f4 = -1; ps(" f4"); img(&s, sizeof s);
    fill(&s, 0x5a, sizeof s); s.f2 = 33LL; ps(" f2="); pi((long long)s.f2); img(&s, sizeof s);
    fill(&s, 0x5a, sizeof s); s.f4 = 5429601LL; ps(" f4="); pi((long long)s.f4); img(&s, sizeof s);
    putchar('\n');
}
static void t31(void) {
    struct T31 s; ps("T31 "); pu(sizeof s); putchar(' '); pu(_Alignof(struct T31));
    fill(&s, 0, sizeof s); s.f0 = -1; ps(" f0"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f1 = -1; ps(" f1"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f2 = -1; ps(" f2"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f3 = -1; ps(" f3"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f4 = -1; ps(" f4"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f5 = -1; ps(" f5"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f6 = -1; ps(" f6"); img(&s, sizeof s);
    fill(&s, 0x5a, sizeof s); s.f2 = 0LL; ps(" f2="); pi((long long)s.f2); img(&s, sizeof s);
    fill(&s, 0x5a, sizeof s); s.f6 = 0LL; ps(" f6="); pi((long long)s.f6); img(&s, sizeof s);
    putchar('\n');
}
static void t32(void) {
    struct T32 s; ps("T32 "); pu(sizeof s); putchar(' '); pu(_Alignof(struct T32));
    fill(&s, 0, sizeof s); s.f2 = -1; ps(" f2"); img(&s, sizeof s);
    fill(&s, 0x5a, sizeof s); s.f2 = 18LL; ps(" f2="); pi((long long)s.f2); img(&s, sizeof s);
    putchar('\n');
}
static void t33(void) {
    union T33 s; ps("T33 "); pu(sizeof s); putchar(' '); pu(_Alignof(union T33));
    fill(&s, 0, sizeof s); s.f1 = -1; ps(" f1"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f2 = -1; ps(" f2"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f3 = -1; ps(" f3"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f4 = -1; ps(" f4"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f5 = -1; ps(" f5"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f6 = -1; ps(" f6"); img(&s, sizeof s);
    fill(&s, 0x5a, sizeof s); s.f1 = 0LL; ps(" f1="); pi((long long)s.f1); img(&s, sizeof s);
    fill(&s, 0x5a, sizeof s); s.f2 = 162LL; ps(" f2="); pi((long long)s.f2); img(&s, sizeof s);
    fill(&s, 0x5a, sizeof s); s.f3 = -589064727600099211LL; ps(" f3="); pi((long long)s.f3); img(&s, sizeof s);
    fill(&s, 0x5a, sizeof s); s.f4 = 10LL; ps(" f4="); pi((long long)s.f4); img(&s, sizeof s);
    fill(&s, 0x5a, sizeof s); s.f5 = 856LL; ps(" f5="); pi((long long)s.f5); img(&s, sizeof s);
    putchar('\n');
}
static void t34(void) {
    struct T34 s; ps("T34 "); pu(sizeof s); putchar(' '); pu(_Alignof(struct T34));
    fill(&s, 0, sizeof s); s.f1 = -1; ps(" f1"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f2 = -1; ps(" f2"); img(&s, sizeof s);
    fill(&s, 0x5a, sizeof s); s.f1 = 1LL; ps(" f1="); pi((long long)s.f1); img(&s, sizeof s);
    putchar('\n');
}
static void t35(void) {
    struct T35 s; ps("T35 "); pu(sizeof s); putchar(' '); pu(_Alignof(struct T35));
    fill(&s, 0, sizeof s); s.f0 = -1; ps(" f0"); img(&s, sizeof s);
    fill(&s, 0x5a, sizeof s); s.f0 = 1LL; ps(" f0="); pi((long long)s.f0); img(&s, sizeof s);
    putchar('\n');
}
static void t36(void) {
    struct T36 s; ps("T36 "); pu(sizeof s); putchar(' '); pu(_Alignof(struct T36));
    fill(&s, 0, sizeof s); s.f0 = -1; ps(" f0"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); fill(&s.f1, 0xff, sizeof s.f1); ps(" f1"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f3 = -1; ps(" f3"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); fill(&s.f4, 0xff, sizeof s.f4); ps(" f4"); img(&s, sizeof s);
    fill(&s, 0x5a, sizeof s); s.f0 = 0LL; ps(" f0="); pi((long long)s.f0); img(&s, sizeof s);
    putchar('\n');
}
static void t37(void) {
    struct T37 s; ps("T37 "); pu(sizeof s); putchar(' '); pu(_Alignof(struct T37));
    fill(&s, 0, sizeof s); s.f0 = -1; ps(" f0"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f1 = -1; ps(" f1"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f2 = -1; ps(" f2"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f3 = -1; ps(" f3"); img(&s, sizeof s);
    fill(&s, 0x5a, sizeof s); s.f0 = 76LL; ps(" f0="); pi((long long)s.f0); img(&s, sizeof s);
    fill(&s, 0x5a, sizeof s); s.f3 = 7514706964LL; ps(" f3="); pi((long long)s.f3); img(&s, sizeof s);
    putchar('\n');
}
static void t38(void) {
    struct T38 s; ps("T38 "); pu(sizeof s); putchar(' '); pu(_Alignof(struct T38));
    fill(&s, 0, sizeof s); s.f0 = -1; ps(" f0"); img(&s, sizeof s);
    putchar('\n');
}
static void t39(void) {
    struct T39 s; ps("T39 "); pu(sizeof s); putchar(' '); pu(_Alignof(struct T39));
    fill(&s, 0, sizeof s); s.f0[0] = -1; ps(" f0"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); fill(&s.f1, 0xff, sizeof s.f1); ps(" f1"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f2 = -1; ps(" f2"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f3 = -1; ps(" f3"); img(&s, sizeof s);
    fill(&s, 0x5a, sizeof s); s.f2 = 31567LL; ps(" f2="); pi((long long)s.f2); img(&s, sizeof s);
    fill(&s, 0x5a, sizeof s); s.f3 = 1LL; ps(" f3="); pi((long long)s.f3); img(&s, sizeof s);
    putchar('\n');
}
static void t40(void) {
    struct T40 s; ps("T40 "); pu(sizeof s); putchar(' '); pu(_Alignof(struct T40));
    fill(&s, 0, sizeof s); s.f0 = -1; ps(" f0"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f1 = -1; ps(" f1"); img(&s, sizeof s);
    fill(&s, 0x5a, sizeof s); s.f0 = 0LL; ps(" f0="); pi((long long)s.f0); img(&s, sizeof s);
    putchar('\n');
}
static void t41(void) {
    union T41 s; ps("T41 "); pu(sizeof s); putchar(' '); pu(_Alignof(union T41));
    fill(&s, 0, sizeof s); s.f0 = -1; ps(" f0"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f1 = -1; ps(" f1"); img(&s, sizeof s);
    fill(&s, 0x5a, sizeof s); s.f0 = 11LL; ps(" f0="); pi((long long)s.f0); img(&s, sizeof s);
    putchar('\n');
}
static void t42(void) {
    union T42 s; ps("T42 "); pu(sizeof s); putchar(' '); pu(_Alignof(union T42));
    fill(&s, 0, sizeof s); s.f0 = -1; ps(" f0"); img(&s, sizeof s);
    putchar('\n');
}
static void t43(void) {
    struct T43 s; ps("T43 "); pu(sizeof s); putchar(' '); pu(_Alignof(struct T43));
    fill(&s, 0, sizeof s); s.f0 = -1; ps(" f0"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f1 = -1; ps(" f1"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f2[0] = -1; ps(" f2"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f3 = -1; ps(" f3"); img(&s, sizeof s);
    fill(&s, 0x5a, sizeof s); s.f0 = -1107092LL; ps(" f0="); pi((long long)s.f0); img(&s, sizeof s);
    fill(&s, 0x5a, sizeof s); s.f3 = -3LL; ps(" f3="); pi((long long)s.f3); img(&s, sizeof s);
    putchar('\n');
}
static void t44(void) {
    struct T44 s; ps("T44 "); pu(sizeof s); putchar(' '); pu(_Alignof(struct T44));
    fill(&s, 0, sizeof s); s.f0 = -1; ps(" f0"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f2 = -1; ps(" f2"); img(&s, sizeof s);
    fill(&s, 0x5a, sizeof s); s.f0 = 4029921558LL; ps(" f0="); pi((long long)s.f0); img(&s, sizeof s);
    fill(&s, 0x5a, sizeof s); s.f2 = 0LL; ps(" f2="); pi((long long)s.f2); img(&s, sizeof s);
    putchar('\n');
}
static void t45(void) {
    struct T45 s; ps("T45 "); pu(sizeof s); putchar(' '); pu(_Alignof(struct T45));
    fill(&s, 0, sizeof s); s.f0 = -1; ps(" f0"); img(&s, sizeof s);
    fill(&s, 0x5a, sizeof s); s.f0 = -80LL; ps(" f0="); pi((long long)s.f0); img(&s, sizeof s);
    putchar('\n');
}
static void t46(void) {
    struct T46 s; ps("T46 "); pu(sizeof s); putchar(' '); pu(_Alignof(struct T46));
    fill(&s, 0, sizeof s); s.f0 = -1; ps(" f0"); img(&s, sizeof s);
    fill(&s, 0x5a, sizeof s); s.f0 = -32857558LL; ps(" f0="); pi((long long)s.f0); img(&s, sizeof s);
    putchar('\n');
}
static void t47(void) {
    struct T47 s; ps("T47 "); pu(sizeof s); putchar(' '); pu(_Alignof(struct T47));
    fill(&s, 0, sizeof s); s.f0 = -1; ps(" f0"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f1 = -1; ps(" f1"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f2 = -1; ps(" f2"); img(&s, sizeof s);
    fill(&s, 0x5a, sizeof s); s.f0 = 21400LL; ps(" f0="); pi((long long)s.f0); img(&s, sizeof s);
    fill(&s, 0x5a, sizeof s); s.f1 = 1LL; ps(" f1="); pi((long long)s.f1); img(&s, sizeof s);
    fill(&s, 0x5a, sizeof s); s.f2 = 6LL; ps(" f2="); pi((long long)s.f2); img(&s, sizeof s);
    putchar('\n');
}
static void t48(void) {
    struct T48 s; ps("T48 "); pu(sizeof s); putchar(' '); pu(_Alignof(struct T48));
    fill(&s, 0, sizeof s); s.f0 = -1; ps(" f0"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f1 = -1; ps(" f1"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f3 = -1; ps(" f3"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f4 = -1; ps(" f4"); img(&s, sizeof s);
    fill(&s, 0x5a, sizeof s); s.f0 = 0LL; ps(" f0="); pi((long long)s.f0); img(&s, sizeof s);
    fill(&s, 0x5a, sizeof s); s.f3 = 1LL; ps(" f3="); pi((long long)s.f3); img(&s, sizeof s);
    putchar('\n');
}
static void t49(void) {
    struct T49 s; ps("T49 "); pu(sizeof s); putchar(' '); pu(_Alignof(struct T49));
    fill(&s, 0, sizeof s); s.f0 = -1; ps(" f0"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f1 = -1; ps(" f1"); img(&s, sizeof s);
    fill(&s, 0x5a, sizeof s); s.f0 = 1742LL; ps(" f0="); pi((long long)s.f0); img(&s, sizeof s);
    fill(&s, 0x5a, sizeof s); s.f1 = 93LL; ps(" f1="); pi((long long)s.f1); img(&s, sizeof s);
    putchar('\n');
}
static void t50(void) {
    union T50 s; ps("T50 "); pu(sizeof s); putchar(' '); pu(_Alignof(union T50));
    fill(&s, 0, sizeof s); s.f0 = -1; ps(" f0"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f1 = -1; ps(" f1"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f2 = -1; ps(" f2"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f3 = -1; ps(" f3"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f4 = -1; ps(" f4"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f5 = -1; ps(" f5"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f6 = -1; ps(" f6"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f7 = -1; ps(" f7"); img(&s, sizeof s);
    fill(&s, 0x5a, sizeof s); s.f0 = -1LL; ps(" f0="); pi((long long)s.f0); img(&s, sizeof s);
    fill(&s, 0x5a, sizeof s); s.f3 = -325444214767337LL; ps(" f3="); pi((long long)s.f3); img(&s, sizeof s);
    fill(&s, 0x5a, sizeof s); s.f4 = 0LL; ps(" f4="); pi((long long)s.f4); img(&s, sizeof s);
    fill(&s, 0x5a, sizeof s); s.f5 = 0LL; ps(" f5="); pi((long long)s.f5); img(&s, sizeof s);
    fill(&s, 0x5a, sizeof s); s.f6 = 14LL; ps(" f6="); pi((long long)s.f6); img(&s, sizeof s);
    fill(&s, 0x5a, sizeof s); s.f7 = 32LL; ps(" f7="); pi((long long)s.f7); img(&s, sizeof s);
    putchar('\n');
}
static void t51(void) {
    struct T51 s; ps("T51 "); pu(sizeof s); putchar(' '); pu(_Alignof(struct T51));
    fill(&s, 0, sizeof s); s.f0 = -1; ps(" f0"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f1 = -1; ps(" f1"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f2 = -1; ps(" f2"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f3 = -1; ps(" f3"); img(&s, sizeof s);
    fill(&s, 0x5a, sizeof s); s.f1 = -6507222461LL; ps(" f1="); pi((long long)s.f1); img(&s, sizeof s);
    fill(&s, 0x5a, sizeof s); s.f2 = 0LL; ps(" f2="); pi((long long)s.f2); img(&s, sizeof s);
    fill(&s, 0x5a, sizeof s); s.f3 = 0LL; ps(" f3="); pi((long long)s.f3); img(&s, sizeof s);
    putchar('\n');
}
static void t52(void) {
    struct T52 s; ps("T52 "); pu(sizeof s); putchar(' '); pu(_Alignof(struct T52));
    fill(&s, 0, sizeof s); fill(&s.f1, 0xff, sizeof s.f1); ps(" f1"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); fill(&s.f2, 0xff, sizeof s.f2); ps(" f2"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f3 = -1; ps(" f3"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f4 = -1; ps(" f4"); img(&s, sizeof s);
    fill(&s, 0x5a, sizeof s); s.f3 = 2433LL; ps(" f3="); pi((long long)s.f3); img(&s, sizeof s);
    fill(&s, 0x5a, sizeof s); s.f4 = 1LL; ps(" f4="); pi((long long)s.f4); img(&s, sizeof s);
    putchar('\n');
}
static void t53(void) {
    struct T53 s; ps("T53 "); pu(sizeof s); putchar(' '); pu(_Alignof(struct T53));
    fill(&s, 0, sizeof s); s.f1 = -1; ps(" f1"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f2 = -1; ps(" f2"); img(&s, sizeof s);
    fill(&s, 0x5a, sizeof s); s.f2 = 1LL; ps(" f2="); pi((long long)s.f2); img(&s, sizeof s);
    putchar('\n');
}
static void t54(void) {
    struct T54 s; ps("T54 "); pu(sizeof s); putchar(' '); pu(_Alignof(struct T54));
    fill(&s, 0, sizeof s); s.f0 = -1; ps(" f0"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f2[0] = -1; ps(" f2"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f3[0] = -1; ps(" f3"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f4 = -1; ps(" f4"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f5 = -1; ps(" f5"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f6 = -1; ps(" f6"); img(&s, sizeof s);
    fill(&s, 0x5a, sizeof s); s.f6 = 1966LL; ps(" f6="); pi((long long)s.f6); img(&s, sizeof s);
    putchar('\n');
}
static void t55(void) {
    struct T55 s; ps("T55 "); pu(sizeof s); putchar(' '); pu(_Alignof(struct T55));
    fill(&s, 0, sizeof s); s.f0 = -1; ps(" f0"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f1 = -1; ps(" f1"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f2 = -1; ps(" f2"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f3 = -1; ps(" f3"); img(&s, sizeof s);
    fill(&s, 0x5a, sizeof s); s.f1 = 1LL; ps(" f1="); pi((long long)s.f1); img(&s, sizeof s);
    fill(&s, 0x5a, sizeof s); s.f2 = 15566LL; ps(" f2="); pi((long long)s.f2); img(&s, sizeof s);
    fill(&s, 0x5a, sizeof s); s.f3 = 0LL; ps(" f3="); pi((long long)s.f3); img(&s, sizeof s);
    putchar('\n');
}
static void t56(void) {
    struct T56 s; ps("T56 "); pu(sizeof s); putchar(' '); pu(_Alignof(struct T56));
    fill(&s, 0, sizeof s); s.f1 = -1; ps(" f1"); img(&s, sizeof s);
    putchar('\n');
}
static void t57(void) {
    struct T57 s; ps("T57 "); pu(sizeof s); putchar(' '); pu(_Alignof(struct T57));
    fill(&s, 0, sizeof s); s.f0 = -1; ps(" f0"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f1 = -1; ps(" f1"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f2 = -1; ps(" f2"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f3 = -1; ps(" f3"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f4[0] = -1; ps(" f4"); img(&s, sizeof s);
    fill(&s, 0x5a, sizeof s); s.f0 = 127LL; ps(" f0="); pi((long long)s.f0); img(&s, sizeof s);
    fill(&s, 0x5a, sizeof s); s.f1 = 1LL; ps(" f1="); pi((long long)s.f1); img(&s, sizeof s);
    fill(&s, 0x5a, sizeof s); s.f2 = 151526LL; ps(" f2="); pi((long long)s.f2); img(&s, sizeof s);
    fill(&s, 0x5a, sizeof s); s.f3 = -1LL; ps(" f3="); pi((long long)s.f3); img(&s, sizeof s);
    putchar('\n');
}
static void t58(void) {
    struct T58 s; ps("T58 "); pu(sizeof s); putchar(' '); pu(_Alignof(struct T58));
    fill(&s, 0, sizeof s); s.f0 = -1; ps(" f0"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f2 = -1; ps(" f2"); img(&s, sizeof s);
    fill(&s, 0x5a, sizeof s); s.f0 = 2LL; ps(" f0="); pi((long long)s.f0); img(&s, sizeof s);
    fill(&s, 0x5a, sizeof s); s.f2 = 0LL; ps(" f2="); pi((long long)s.f2); img(&s, sizeof s);
    putchar('\n');
}
static void t59(void) {
    struct T59 s; ps("T59 "); pu(sizeof s); putchar(' '); pu(_Alignof(struct T59));
    fill(&s, 0, sizeof s); s.f0[0] = -1; ps(" f0"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); fill(&s.f1, 0xff, sizeof s.f1); ps(" f1"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f2 = -1; ps(" f2"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f4 = -1; ps(" f4"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f6 = -1; ps(" f6"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f7 = -1; ps(" f7"); img(&s, sizeof s);
    fill(&s, 0x5a, sizeof s); s.f2 = 313949542LL; ps(" f2="); pi((long long)s.f2); img(&s, sizeof s);
    fill(&s, 0x5a, sizeof s); s.f6 = 1LL; ps(" f6="); pi((long long)s.f6); img(&s, sizeof s);
    putchar('\n');
}
int main(void) {
    t0();
    t1();
    t2();
    t3();
    t4();
    t5();
    t6();
    t7();
    t8();
    t9();
    t10();
    t11();
    t12();
    t13();
    t14();
    t15();
    t16();
    t17();
    t18();
    t19();
    t20();
    t21();
    t22();
    t23();
    t24();
    t25();
    t26();
    t27();
    t28();
    t29();
    t30();
    t31();
    t32();
    t33();
    t34();
    t35();
    t36();
    t37();
    t38();
    t39();
    t40();
    t41();
    t42();
    t43();
    t44();
    t45();
    t46();
    t47();
    t48();
    t49();
    t50();
    t51();
    t52();
    t53();
    t54();
    t55();
    t56();
    t57();
    t58();
    t59();
    return 0;
}
