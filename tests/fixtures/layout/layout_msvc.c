int putchar(int);
static void ps(const char *s) { while (*s) putchar(*s++); }
static void pu(unsigned long long v) { char b[24]; int n = 0; do b[n++] = (char)('0' + v % 10); while (v /= 10); while (n) putchar(b[--n]); }
static void pi(long long v) { if (v < 0) { putchar('-'); pu(0ull - (unsigned long long)v); } else pu((unsigned long long)v); }
static void img(const void *p, unsigned long long n) { const unsigned char *b = (const unsigned char *)p; putchar(':'); while (n--) { putchar("0123456789abcdef"[*b >> 4]); putchar("0123456789abcdef"[*b & 15]); b++; } }
static void fill(void *p, int v, unsigned long long n) { unsigned char *b = (unsigned char *)p; while (n--) *b++ = (unsigned char)v; }
#pragma pack(push, 2)
struct T0 { unsigned f0[3]; signed char : 0; unsigned long long : 0; };
#pragma pack(pop)
#pragma pack(push, 4)
union T1 { unsigned long long f0 : 16; _Bool f1 : 1; };
#pragma pack(pop)
#pragma pack(push, 4)
struct T2 { unsigned : 0; unsigned short f1 : 4; int f2 : 4; unsigned long long f3 : 59; signed char f4 : 6; };
#pragma pack(pop)
#pragma pack(push, 4)
struct T3 { unsigned long long f0 : 20; char f1[3]; };
#pragma pack(pop)
#pragma pack(push, 2)
struct T4 { int f0; long long : 0; unsigned short f2 : 2; _Bool f3 : 1; int f4 : 11; char f5 : 4; };
#pragma pack(pop)
union T5 { unsigned f0 : 28; unsigned f1; short f2 : 8; struct T2 f3; char f4 : 6; unsigned long long f5; long long f6; unsigned long long f7 : 14; };
#pragma pack(push, 2)
struct T6 { long long f0 : 7; unsigned char : 1; short f2[2]; _Alignas(8) int f3; };
#pragma pack(pop)
struct T7 { signed char f0 : 5; unsigned char f1 : 6; unsigned long long f2 : 12; unsigned long long f3 : 29; short f4; };
#pragma pack(push, 8)
struct T8 { short f0 : 1; long long f1 : 45; int f2; short f3 : 16; _Bool f4 : 1; signed char f5[2]; short : 11; };
#pragma pack(pop)
#pragma pack(push, 2)
struct T9 { struct T2 f0; char f1 : 3; long long f2; unsigned char f3 : 2; struct T3 f4; char f5 : 4; unsigned short f6 : 2; };
#pragma pack(pop)
#pragma pack(push, 2)
struct T10 { unsigned long long f0 : 3; _Bool : 0; unsigned char f2 : 2; unsigned long long f3 : 14; short : 0; signed char f5 : 2; struct T4 f6; };
#pragma pack(pop)
struct T11 { unsigned long long f0; unsigned short f1; long long : 0; long long : 55; unsigned short f4; int f5 : 30; signed char f6 : 4; unsigned long long f7 : 46; };
struct T12 { long long f0; unsigned short : 0; short f2[2]; unsigned short f3 : 9; unsigned short f4 : 16; unsigned short f5 : 14; _Alignas(2) char f6; signed char : 0; };
#pragma pack(push, 4)
struct T13 { unsigned short f0[3]; short f1; unsigned char f2 : 5; unsigned short f3 : 9; unsigned short : 0; unsigned long long f5 : 61; unsigned f6; };
#pragma pack(pop)
struct T14 { short f0 : 5; char f1[3]; unsigned short : 0; unsigned f3; };
#pragma pack(push, 4)
struct T15 { long long f0 : 1; };
#pragma pack(pop)
#pragma pack(push, 1)
struct T16 { unsigned short f0 : 11; };
#pragma pack(pop)
#pragma pack(push, 1)
struct T17 { short : 0; unsigned short f1[3]; char : 4; struct T12 f3; };
#pragma pack(pop)
#pragma pack(push, 4)
struct T18 { unsigned char : 3; unsigned long long f1; unsigned char f2; char f3; short : 0; int f5; char f6; long long f7 : 9; };
#pragma pack(pop)
#pragma pack(push, 2)
union T19 { long long : 0; unsigned short f1 : 8; };
#pragma pack(pop)
struct T20 { union T1 f0; _Bool : 1; };
struct T21 { _Bool f0 : 1; unsigned short f1[1]; long long f2 : 60; unsigned long long : 61; long long f4 : 58; };
union T22 { struct T11 f0; unsigned char f1; };
#pragma pack(push, 8)
struct T23 { _Alignas(8) long long f0; long long f1; unsigned : 22; int f3[1]; };
#pragma pack(pop)
#pragma pack(push, 4)
union T24 { unsigned short f0 : 3; union T1 f1; unsigned short f2[2]; unsigned f3 : 24; char f4; };
#pragma pack(pop)
#pragma pack(push, 4)
struct T25 { char f0; union T1 f1; };
#pragma pack(pop)
#pragma pack(push, 4)
union T26 { unsigned f0 : 17; unsigned short f1 : 4; signed char f2 : 8; int f3; short f4 : 3; struct T0 f5; unsigned f6 : 14; char f7 : 6; };
#pragma pack(pop)
#pragma pack(push, 2)
struct T27 { signed char f0 : 7; _Alignas(2) unsigned short f1; unsigned f2; _Bool f3 : 1; };
#pragma pack(pop)
#pragma pack(push, 4)
struct T28 { short f0 : 4; long long f1 : 1; };
#pragma pack(pop)
#pragma pack(push, 8)
struct T29 { unsigned char f0; signed char f1 : 4; _Bool : 0; unsigned long long f3 : 41; long long f4 : 4; };
#pragma pack(pop)
struct T30 { short : 14; int fz; };
#pragma pack(push, 2)
struct T31 { short f0 : 14; short f1 : 3; struct T14 f2; short f3 : 4; _Bool f4 : 1; _Bool : 0; };
#pragma pack(pop)
union T32 { _Bool : 0; int fz; };
struct T33 { unsigned f0 : 21; unsigned char : 8; unsigned f2; };
#pragma pack(push, 4)
union T34 { signed char f0 : 2; unsigned : 28; long long f2 : 58; long long : 52; char f4 : 1; };
#pragma pack(pop)
struct T35 { int f0; struct T4 f1; char f2[3]; long long f3; unsigned f4[2]; };
union T36 { unsigned char f0; int f1; struct T4 f2; char f3 : 3; signed char : 0; };
#pragma pack(push, 4)
struct T37 { unsigned char f0 : 4; unsigned short f1 : 12; short f2 : 8; _Bool f3 : 1; unsigned long long f4 : 13; signed char f5 : 4; int : 8; _Bool f7 : 1; };
#pragma pack(pop)
#pragma pack(push, 2)
struct T38 { _Bool f0 : 1; short f1 : 2; char f2 : 4; int : 20; _Alignas(1) char f4; union T34 f5; signed char f6; _Alignas(8) unsigned short f7; };
#pragma pack(pop)
#pragma pack(push, 2)
struct T39 { short f0 : 1; unsigned : 0; _Bool f2 : 1; char : 0; unsigned f4 : 24; unsigned char f5 : 3; };
#pragma pack(pop)
struct T40 { unsigned short f0 : 16; signed char f1; unsigned char f2; _Bool f3 : 1; };
#pragma pack(push, 4)
struct T41 { unsigned : 0; short f1 : 2; signed char f2 : 5; };
#pragma pack(pop)
struct T42 { int : 2; long long f1 : 59; unsigned : 0; unsigned : 3; unsigned char : 2; unsigned long long f5; char f6; };
#pragma pack(push, 1)
struct T43 { unsigned short f0; short f1 : 9; _Bool f2 : 1; short f3 : 8; short f4 : 9; unsigned f5 : 8; int f6; signed char f7 : 7; };
#pragma pack(pop)
struct T44 { _Bool : 0; int f1; char f2 : 5; _Bool f3 : 1; short f4 : 14; char : 1; };
#pragma pack(push, 4)
struct T45 { signed char f0 : 7; unsigned char f1 : 8; _Alignas(4) short f2; unsigned short : 1; unsigned long long f4; };
#pragma pack(pop)
#pragma pack(push, 1)
struct T46 { short : 0; char f1 : 4; signed char f2 : 4; unsigned long long f3; _Bool : 1; char f5; unsigned long long : 60; long long f7 : 34; };
#pragma pack(pop)
#pragma pack(push, 8)
struct T47 { char f0 : 7; struct T13 f1; signed char f2[2]; struct T10 f3; int f4 : 16; };
#pragma pack(pop)
#pragma pack(push, 8)
struct T48 { long long f0; union T19 f1; short f2 : 6; signed char f3 : 6; char : 0; _Alignas(16) char f5; int f6 : 5; unsigned : 8; };
#pragma pack(pop)
union T49 { long long : 0; short : 9; unsigned short f2[2]; int f3; unsigned short f4[2]; };
struct T50 { int : 0; _Bool f1 : 1; };
#pragma pack(push, 8)
struct T51 { _Alignas(16) unsigned short f0; signed char f1 : 3; int f2; unsigned char f3 : 4; };
#pragma pack(pop)
#pragma pack(push, 8)
struct T52 { unsigned long long f0; unsigned f1; unsigned f2[1]; unsigned short f3 : 6; long long f4 : 5; unsigned long long f5 : 42; _Bool f6 : 1; unsigned long long f7 : 20; };
#pragma pack(pop)
#pragma pack(push, 8)
struct T53 { unsigned char f0 : 5; unsigned char f1[2]; unsigned char f2; unsigned : 0; signed char : 8; unsigned f5; };
#pragma pack(pop)
#pragma pack(push, 8)
struct T54 { unsigned char f0 : 1; };
#pragma pack(pop)
#pragma pack(push, 4)
struct T55 { short f0; _Bool f1 : 1; unsigned f2 : 7; unsigned f3; unsigned : 0; unsigned f5 : 12; char : 2; };
#pragma pack(pop)
#pragma pack(push, 1)
struct T56 { unsigned long long f0 : 59; unsigned long long f1 : 48; union T32 f2; signed char f3; };
#pragma pack(pop)
struct T57 { char f0 : 7; int : 0; unsigned short f2; short f3; unsigned char f4 : 4; unsigned char f5 : 7; unsigned long long f6; };
struct T58 { unsigned short f0; unsigned short f1 : 11; _Bool f2 : 1; unsigned f3 : 21; _Alignas(8) unsigned long long f4; short f5; signed char f6 : 3; };
#pragma pack(push, 8)
struct T59 { unsigned long long f0 : 12; short f1 : 3; signed char f2 : 4; long long f3 : 19; long long f4 : 1; long long f5; long long f6 : 10; };
#pragma pack(pop)
static void t0(void) {
    struct T0 s; ps("T0 "); pu(sizeof s); putchar(' '); pu(_Alignof(struct T0));
    fill(&s, 0, sizeof s); s.f0[0] = -1; ps(" f0"); img(&s, sizeof s);
    putchar('\n');
}
static void t1(void) {
    union T1 s; ps("T1 "); pu(sizeof s); putchar(' '); pu(_Alignof(union T1));
    fill(&s, 0, sizeof s); s.f0 = -1; ps(" f0"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f1 = -1; ps(" f1"); img(&s, sizeof s);
    fill(&s, 0x5a, sizeof s); s.f0 = 8108LL; ps(" f0="); pi((long long)s.f0); img(&s, sizeof s);
    fill(&s, 0x5a, sizeof s); s.f1 = 0LL; ps(" f1="); pi((long long)s.f1); img(&s, sizeof s);
    putchar('\n');
}
static void t2(void) {
    struct T2 s; ps("T2 "); pu(sizeof s); putchar(' '); pu(_Alignof(struct T2));
    fill(&s, 0, sizeof s); s.f1 = -1; ps(" f1"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f2 = -1; ps(" f2"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f3 = -1; ps(" f3"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f4 = -1; ps(" f4"); img(&s, sizeof s);
    fill(&s, 0x5a, sizeof s); s.f1 = 6LL; ps(" f1="); pi((long long)s.f1); img(&s, sizeof s);
    fill(&s, 0x5a, sizeof s); s.f2 = 0LL; ps(" f2="); pi((long long)s.f2); img(&s, sizeof s);
    fill(&s, 0x5a, sizeof s); s.f3 = 281424123352246816LL; ps(" f3="); pi((long long)s.f3); img(&s, sizeof s);
    fill(&s, 0x5a, sizeof s); s.f4 = -13LL; ps(" f4="); pi((long long)s.f4); img(&s, sizeof s);
    putchar('\n');
}
static void t3(void) {
    struct T3 s; ps("T3 "); pu(sizeof s); putchar(' '); pu(_Alignof(struct T3));
    fill(&s, 0, sizeof s); s.f0 = -1; ps(" f0"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f1[0] = -1; ps(" f1"); img(&s, sizeof s);
    fill(&s, 0x5a, sizeof s); s.f0 = 884365LL; ps(" f0="); pi((long long)s.f0); img(&s, sizeof s);
    putchar('\n');
}
static void t4(void) {
    struct T4 s; ps("T4 "); pu(sizeof s); putchar(' '); pu(_Alignof(struct T4));
    fill(&s, 0, sizeof s); s.f0 = -1; ps(" f0"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f2 = -1; ps(" f2"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f3 = -1; ps(" f3"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f4 = -1; ps(" f4"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f5 = -1; ps(" f5"); img(&s, sizeof s);
    fill(&s, 0x5a, sizeof s); s.f2 = 2LL; ps(" f2="); pi((long long)s.f2); img(&s, sizeof s);
    fill(&s, 0x5a, sizeof s); s.f3 = 1LL; ps(" f3="); pi((long long)s.f3); img(&s, sizeof s);
    fill(&s, 0x5a, sizeof s); s.f4 = -12LL; ps(" f4="); pi((long long)s.f4); img(&s, sizeof s);
    fill(&s, 0x5a, sizeof s); s.f5 = 7LL; ps(" f5="); pi((long long)s.f5); img(&s, sizeof s);
    putchar('\n');
}
static void t5(void) {
    union T5 s; ps("T5 "); pu(sizeof s); putchar(' '); pu(_Alignof(union T5));
    fill(&s, 0, sizeof s); s.f0 = -1; ps(" f0"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f1 = -1; ps(" f1"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f2 = -1; ps(" f2"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); fill(&s.f3, 0xff, sizeof s.f3); ps(" f3"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f4 = -1; ps(" f4"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f5 = -1; ps(" f5"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f6 = -1; ps(" f6"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f7 = -1; ps(" f7"); img(&s, sizeof s);
    fill(&s, 0x5a, sizeof s); s.f0 = 149476169LL; ps(" f0="); pi((long long)s.f0); img(&s, sizeof s);
    fill(&s, 0x5a, sizeof s); s.f2 = -124LL; ps(" f2="); pi((long long)s.f2); img(&s, sizeof s);
    fill(&s, 0x5a, sizeof s); s.f4 = 13LL; ps(" f4="); pi((long long)s.f4); img(&s, sizeof s);
    fill(&s, 0x5a, sizeof s); s.f7 = 13121LL; ps(" f7="); pi((long long)s.f7); img(&s, sizeof s);
    putchar('\n');
}
static void t6(void) {
    struct T6 s; ps("T6 "); pu(sizeof s); putchar(' '); pu(_Alignof(struct T6));
    fill(&s, 0, sizeof s); s.f0 = -1; ps(" f0"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f2[0] = -1; ps(" f2"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f3 = -1; ps(" f3"); img(&s, sizeof s);
    fill(&s, 0x5a, sizeof s); s.f0 = 9LL; ps(" f0="); pi((long long)s.f0); img(&s, sizeof s);
    putchar('\n');
}
static void t7(void) {
    struct T7 s; ps("T7 "); pu(sizeof s); putchar(' '); pu(_Alignof(struct T7));
    fill(&s, 0, sizeof s); s.f0 = -1; ps(" f0"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f1 = -1; ps(" f1"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f2 = -1; ps(" f2"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f3 = -1; ps(" f3"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f4 = -1; ps(" f4"); img(&s, sizeof s);
    fill(&s, 0x5a, sizeof s); s.f0 = 7LL; ps(" f0="); pi((long long)s.f0); img(&s, sizeof s);
    fill(&s, 0x5a, sizeof s); s.f1 = 3LL; ps(" f1="); pi((long long)s.f1); img(&s, sizeof s);
    fill(&s, 0x5a, sizeof s); s.f2 = 2139LL; ps(" f2="); pi((long long)s.f2); img(&s, sizeof s);
    fill(&s, 0x5a, sizeof s); s.f3 = 353975088LL; ps(" f3="); pi((long long)s.f3); img(&s, sizeof s);
    putchar('\n');
}
static void t8(void) {
    struct T8 s; ps("T8 "); pu(sizeof s); putchar(' '); pu(_Alignof(struct T8));
    fill(&s, 0, sizeof s); s.f0 = -1; ps(" f0"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f1 = -1; ps(" f1"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f2 = -1; ps(" f2"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f3 = -1; ps(" f3"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f4 = -1; ps(" f4"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f5[0] = -1; ps(" f5"); img(&s, sizeof s);
    fill(&s, 0x5a, sizeof s); s.f0 = 0LL; ps(" f0="); pi((long long)s.f0); img(&s, sizeof s);
    fill(&s, 0x5a, sizeof s); s.f1 = 7850929725192LL; ps(" f1="); pi((long long)s.f1); img(&s, sizeof s);
    fill(&s, 0x5a, sizeof s); s.f3 = 26235LL; ps(" f3="); pi((long long)s.f3); img(&s, sizeof s);
    fill(&s, 0x5a, sizeof s); s.f4 = 1LL; ps(" f4="); pi((long long)s.f4); img(&s, sizeof s);
    putchar('\n');
}
static void t9(void) {
    struct T9 s; ps("T9 "); pu(sizeof s); putchar(' '); pu(_Alignof(struct T9));
    fill(&s, 0, sizeof s); fill(&s.f0, 0xff, sizeof s.f0); ps(" f0"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f1 = -1; ps(" f1"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f2 = -1; ps(" f2"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f3 = -1; ps(" f3"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); fill(&s.f4, 0xff, sizeof s.f4); ps(" f4"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f5 = -1; ps(" f5"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f6 = -1; ps(" f6"); img(&s, sizeof s);
    fill(&s, 0x5a, sizeof s); s.f1 = 1LL; ps(" f1="); pi((long long)s.f1); img(&s, sizeof s);
    fill(&s, 0x5a, sizeof s); s.f3 = 1LL; ps(" f3="); pi((long long)s.f3); img(&s, sizeof s);
    fill(&s, 0x5a, sizeof s); s.f5 = -2LL; ps(" f5="); pi((long long)s.f5); img(&s, sizeof s);
    fill(&s, 0x5a, sizeof s); s.f6 = 2LL; ps(" f6="); pi((long long)s.f6); img(&s, sizeof s);
    putchar('\n');
}
static void t10(void) {
    struct T10 s; ps("T10 "); pu(sizeof s); putchar(' '); pu(_Alignof(struct T10));
    fill(&s, 0, sizeof s); s.f0 = -1; ps(" f0"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f2 = -1; ps(" f2"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f3 = -1; ps(" f3"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f5 = -1; ps(" f5"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); fill(&s.f6, 0xff, sizeof s.f6); ps(" f6"); img(&s, sizeof s);
    fill(&s, 0x5a, sizeof s); s.f0 = 2LL; ps(" f0="); pi((long long)s.f0); img(&s, sizeof s);
    fill(&s, 0x5a, sizeof s); s.f2 = 2LL; ps(" f2="); pi((long long)s.f2); img(&s, sizeof s);
    fill(&s, 0x5a, sizeof s); s.f3 = 1861LL; ps(" f3="); pi((long long)s.f3); img(&s, sizeof s);
    fill(&s, 0x5a, sizeof s); s.f5 = 1LL; ps(" f5="); pi((long long)s.f5); img(&s, sizeof s);
    putchar('\n');
}
static void t11(void) {
    struct T11 s; ps("T11 "); pu(sizeof s); putchar(' '); pu(_Alignof(struct T11));
    fill(&s, 0, sizeof s); s.f0 = -1; ps(" f0"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f1 = -1; ps(" f1"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f4 = -1; ps(" f4"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f5 = -1; ps(" f5"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f6 = -1; ps(" f6"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f7 = -1; ps(" f7"); img(&s, sizeof s);
    fill(&s, 0x5a, sizeof s); s.f5 = 485872609LL; ps(" f5="); pi((long long)s.f5); img(&s, sizeof s);
    fill(&s, 0x5a, sizeof s); s.f6 = -1LL; ps(" f6="); pi((long long)s.f6); img(&s, sizeof s);
    fill(&s, 0x5a, sizeof s); s.f7 = 51504054460545LL; ps(" f7="); pi((long long)s.f7); img(&s, sizeof s);
    putchar('\n');
}
static void t12(void) {
    struct T12 s; ps("T12 "); pu(sizeof s); putchar(' '); pu(_Alignof(struct T12));
    fill(&s, 0, sizeof s); s.f0 = -1; ps(" f0"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f2[0] = -1; ps(" f2"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f3 = -1; ps(" f3"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f4 = -1; ps(" f4"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f5 = -1; ps(" f5"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f6 = -1; ps(" f6"); img(&s, sizeof s);
    fill(&s, 0x5a, sizeof s); s.f3 = 432LL; ps(" f3="); pi((long long)s.f3); img(&s, sizeof s);
    fill(&s, 0x5a, sizeof s); s.f4 = 11725LL; ps(" f4="); pi((long long)s.f4); img(&s, sizeof s);
    fill(&s, 0x5a, sizeof s); s.f5 = 8812LL; ps(" f5="); pi((long long)s.f5); img(&s, sizeof s);
    putchar('\n');
}
static void t13(void) {
    struct T13 s; ps("T13 "); pu(sizeof s); putchar(' '); pu(_Alignof(struct T13));
    fill(&s, 0, sizeof s); s.f0[0] = -1; ps(" f0"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f1 = -1; ps(" f1"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f2 = -1; ps(" f2"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f3 = -1; ps(" f3"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f5 = -1; ps(" f5"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f6 = -1; ps(" f6"); img(&s, sizeof s);
    fill(&s, 0x5a, sizeof s); s.f2 = 13LL; ps(" f2="); pi((long long)s.f2); img(&s, sizeof s);
    fill(&s, 0x5a, sizeof s); s.f3 = 18LL; ps(" f3="); pi((long long)s.f3); img(&s, sizeof s);
    fill(&s, 0x5a, sizeof s); s.f5 = 490136197951273884LL; ps(" f5="); pi((long long)s.f5); img(&s, sizeof s);
    putchar('\n');
}
static void t14(void) {
    struct T14 s; ps("T14 "); pu(sizeof s); putchar(' '); pu(_Alignof(struct T14));
    fill(&s, 0, sizeof s); s.f0 = -1; ps(" f0"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f1[0] = -1; ps(" f1"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f3 = -1; ps(" f3"); img(&s, sizeof s);
    fill(&s, 0x5a, sizeof s); s.f0 = -4LL; ps(" f0="); pi((long long)s.f0); img(&s, sizeof s);
    putchar('\n');
}
static void t15(void) {
    struct T15 s; ps("T15 "); pu(sizeof s); putchar(' '); pu(_Alignof(struct T15));
    fill(&s, 0, sizeof s); s.f0 = -1; ps(" f0"); img(&s, sizeof s);
    fill(&s, 0x5a, sizeof s); s.f0 = 0LL; ps(" f0="); pi((long long)s.f0); img(&s, sizeof s);
    putchar('\n');
}
static void t16(void) {
    struct T16 s; ps("T16 "); pu(sizeof s); putchar(' '); pu(_Alignof(struct T16));
    fill(&s, 0, sizeof s); s.f0 = -1; ps(" f0"); img(&s, sizeof s);
    fill(&s, 0x5a, sizeof s); s.f0 = 1944LL; ps(" f0="); pi((long long)s.f0); img(&s, sizeof s);
    putchar('\n');
}
static void t17(void) {
    struct T17 s; ps("T17 "); pu(sizeof s); putchar(' '); pu(_Alignof(struct T17));
    fill(&s, 0, sizeof s); s.f1[0] = -1; ps(" f1"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); fill(&s.f3, 0xff, sizeof s.f3); ps(" f3"); img(&s, sizeof s);
    putchar('\n');
}
static void t18(void) {
    struct T18 s; ps("T18 "); pu(sizeof s); putchar(' '); pu(_Alignof(struct T18));
    fill(&s, 0, sizeof s); s.f1 = -1; ps(" f1"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f2 = -1; ps(" f2"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f3 = -1; ps(" f3"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f5 = -1; ps(" f5"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f6 = -1; ps(" f6"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f7 = -1; ps(" f7"); img(&s, sizeof s);
    fill(&s, 0x5a, sizeof s); s.f7 = 2LL; ps(" f7="); pi((long long)s.f7); img(&s, sizeof s);
    putchar('\n');
}
static void t19(void) {
    union T19 s; ps("T19 "); pu(sizeof s); putchar(' '); pu(_Alignof(union T19));
    fill(&s, 0, sizeof s); s.f1 = -1; ps(" f1"); img(&s, sizeof s);
    fill(&s, 0x5a, sizeof s); s.f1 = 235LL; ps(" f1="); pi((long long)s.f1); img(&s, sizeof s);
    putchar('\n');
}
static void t20(void) {
    struct T20 s; ps("T20 "); pu(sizeof s); putchar(' '); pu(_Alignof(struct T20));
    fill(&s, 0, sizeof s); fill(&s.f0, 0xff, sizeof s.f0); ps(" f0"); img(&s, sizeof s);
    putchar('\n');
}
static void t21(void) {
    struct T21 s; ps("T21 "); pu(sizeof s); putchar(' '); pu(_Alignof(struct T21));
    fill(&s, 0, sizeof s); s.f0 = -1; ps(" f0"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f1[0] = -1; ps(" f1"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f2 = -1; ps(" f2"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f4 = -1; ps(" f4"); img(&s, sizeof s);
    fill(&s, 0x5a, sizeof s); s.f0 = 1LL; ps(" f0="); pi((long long)s.f0); img(&s, sizeof s);
    fill(&s, 0x5a, sizeof s); s.f2 = -439836258774729505LL; ps(" f2="); pi((long long)s.f2); img(&s, sizeof s);
    fill(&s, 0x5a, sizeof s); s.f4 = -83633727535022509LL; ps(" f4="); pi((long long)s.f4); img(&s, sizeof s);
    putchar('\n');
}
static void t22(void) {
    union T22 s; ps("T22 "); pu(sizeof s); putchar(' '); pu(_Alignof(union T22));
    fill(&s, 0, sizeof s); fill(&s.f0, 0xff, sizeof s.f0); ps(" f0"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f1 = -1; ps(" f1"); img(&s, sizeof s);
    putchar('\n');
}
static void t23(void) {
    struct T23 s; ps("T23 "); pu(sizeof s); putchar(' '); pu(_Alignof(struct T23));
    fill(&s, 0, sizeof s); s.f0 = -1; ps(" f0"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f1 = -1; ps(" f1"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f3[0] = -1; ps(" f3"); img(&s, sizeof s);
    putchar('\n');
}
static void t24(void) {
    union T24 s; ps("T24 "); pu(sizeof s); putchar(' '); pu(_Alignof(union T24));
    fill(&s, 0, sizeof s); s.f0 = -1; ps(" f0"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); fill(&s.f1, 0xff, sizeof s.f1); ps(" f1"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f2[0] = -1; ps(" f2"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f3 = -1; ps(" f3"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f4 = -1; ps(" f4"); img(&s, sizeof s);
    fill(&s, 0x5a, sizeof s); s.f0 = 6LL; ps(" f0="); pi((long long)s.f0); img(&s, sizeof s);
    fill(&s, 0x5a, sizeof s); s.f3 = 14352828LL; ps(" f3="); pi((long long)s.f3); img(&s, sizeof s);
    putchar('\n');
}
static void t25(void) {
    struct T25 s; ps("T25 "); pu(sizeof s); putchar(' '); pu(_Alignof(struct T25));
    fill(&s, 0, sizeof s); s.f0 = -1; ps(" f0"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); fill(&s.f1, 0xff, sizeof s.f1); ps(" f1"); img(&s, sizeof s);
    putchar('\n');
}
static void t26(void) {
    union T26 s; ps("T26 "); pu(sizeof s); putchar(' '); pu(_Alignof(union T26));
    fill(&s, 0, sizeof s); s.f0 = -1; ps(" f0"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f1 = -1; ps(" f1"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f2 = -1; ps(" f2"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f3 = -1; ps(" f3"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f4 = -1; ps(" f4"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); fill(&s.f5, 0xff, sizeof s.f5); ps(" f5"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f6 = -1; ps(" f6"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f7 = -1; ps(" f7"); img(&s, sizeof s);
    fill(&s, 0x5a, sizeof s); s.f0 = 62564LL; ps(" f0="); pi((long long)s.f0); img(&s, sizeof s);
    fill(&s, 0x5a, sizeof s); s.f1 = 5LL; ps(" f1="); pi((long long)s.f1); img(&s, sizeof s);
    fill(&s, 0x5a, sizeof s); s.f2 = -12LL; ps(" f2="); pi((long long)s.f2); img(&s, sizeof s);
    fill(&s, 0x5a, sizeof s); s.f4 = -1LL; ps(" f4="); pi((long long)s.f4); img(&s, sizeof s);
    fill(&s, 0x5a, sizeof s); s.f6 = 11082LL; ps(" f6="); pi((long long)s.f6); img(&s, sizeof s);
    fill(&s, 0x5a, sizeof s); s.f7 = 1LL; ps(" f7="); pi((long long)s.f7); img(&s, sizeof s);
    putchar('\n');
}
static void t27(void) {
    struct T27 s; ps("T27 "); pu(sizeof s); putchar(' '); pu(_Alignof(struct T27));
    fill(&s, 0, sizeof s); s.f0 = -1; ps(" f0"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f1 = -1; ps(" f1"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f2 = -1; ps(" f2"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f3 = -1; ps(" f3"); img(&s, sizeof s);
    fill(&s, 0x5a, sizeof s); s.f0 = -8LL; ps(" f0="); pi((long long)s.f0); img(&s, sizeof s);
    fill(&s, 0x5a, sizeof s); s.f3 = 1LL; ps(" f3="); pi((long long)s.f3); img(&s, sizeof s);
    putchar('\n');
}
static void t28(void) {
    struct T28 s; ps("T28 "); pu(sizeof s); putchar(' '); pu(_Alignof(struct T28));
    fill(&s, 0, sizeof s); s.f0 = -1; ps(" f0"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f1 = -1; ps(" f1"); img(&s, sizeof s);
    fill(&s, 0x5a, sizeof s); s.f0 = 4LL; ps(" f0="); pi((long long)s.f0); img(&s, sizeof s);
    fill(&s, 0x5a, sizeof s); s.f1 = 0LL; ps(" f1="); pi((long long)s.f1); img(&s, sizeof s);
    putchar('\n');
}
static void t29(void) {
    struct T29 s; ps("T29 "); pu(sizeof s); putchar(' '); pu(_Alignof(struct T29));
    fill(&s, 0, sizeof s); s.f0 = -1; ps(" f0"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f1 = -1; ps(" f1"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f3 = -1; ps(" f3"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f4 = -1; ps(" f4"); img(&s, sizeof s);
    fill(&s, 0x5a, sizeof s); s.f1 = -4LL; ps(" f1="); pi((long long)s.f1); img(&s, sizeof s);
    fill(&s, 0x5a, sizeof s); s.f3 = 1068947669964LL; ps(" f3="); pi((long long)s.f3); img(&s, sizeof s);
    fill(&s, 0x5a, sizeof s); s.f4 = 4LL; ps(" f4="); pi((long long)s.f4); img(&s, sizeof s);
    putchar('\n');
}
static void t30(void) {
    struct T30 s; ps("T30 "); pu(sizeof s); putchar(' '); pu(_Alignof(struct T30));
    fill(&s, 0, sizeof s); s.fz = -1; ps(" fz"); img(&s, sizeof s);
    putchar('\n');
}
static void t31(void) {
    struct T31 s; ps("T31 "); pu(sizeof s); putchar(' '); pu(_Alignof(struct T31));
    fill(&s, 0, sizeof s); s.f0 = -1; ps(" f0"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f1 = -1; ps(" f1"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); fill(&s.f2, 0xff, sizeof s.f2); ps(" f2"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f3 = -1; ps(" f3"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f4 = -1; ps(" f4"); img(&s, sizeof s);
    fill(&s, 0x5a, sizeof s); s.f0 = -1698LL; ps(" f0="); pi((long long)s.f0); img(&s, sizeof s);
    fill(&s, 0x5a, sizeof s); s.f1 = -2LL; ps(" f1="); pi((long long)s.f1); img(&s, sizeof s);
    fill(&s, 0x5a, sizeof s); s.f3 = 0LL; ps(" f3="); pi((long long)s.f3); img(&s, sizeof s);
    fill(&s, 0x5a, sizeof s); s.f4 = 0LL; ps(" f4="); pi((long long)s.f4); img(&s, sizeof s);
    putchar('\n');
}
static void t32(void) {
    union T32 s; ps("T32 "); pu(sizeof s); putchar(' '); pu(_Alignof(union T32));
    fill(&s, 0, sizeof s); s.fz = -1; ps(" fz"); img(&s, sizeof s);
    putchar('\n');
}
static void t33(void) {
    struct T33 s; ps("T33 "); pu(sizeof s); putchar(' '); pu(_Alignof(struct T33));
    fill(&s, 0, sizeof s); s.f0 = -1; ps(" f0"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f2 = -1; ps(" f2"); img(&s, sizeof s);
    fill(&s, 0x5a, sizeof s); s.f0 = 332864LL; ps(" f0="); pi((long long)s.f0); img(&s, sizeof s);
    putchar('\n');
}
static void t34(void) {
    union T34 s; ps("T34 "); pu(sizeof s); putchar(' '); pu(_Alignof(union T34));
    fill(&s, 0, sizeof s); s.f0 = -1; ps(" f0"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f2 = -1; ps(" f2"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f4 = -1; ps(" f4"); img(&s, sizeof s);
    fill(&s, 0x5a, sizeof s); s.f0 = -1LL; ps(" f0="); pi((long long)s.f0); img(&s, sizeof s);
    fill(&s, 0x5a, sizeof s); s.f2 = 68402776001964495LL; ps(" f2="); pi((long long)s.f2); img(&s, sizeof s);
    fill(&s, 0x5a, sizeof s); s.f4 = 0LL; ps(" f4="); pi((long long)s.f4); img(&s, sizeof s);
    putchar('\n');
}
static void t35(void) {
    struct T35 s; ps("T35 "); pu(sizeof s); putchar(' '); pu(_Alignof(struct T35));
    fill(&s, 0, sizeof s); s.f0 = -1; ps(" f0"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); fill(&s.f1, 0xff, sizeof s.f1); ps(" f1"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f2[0] = -1; ps(" f2"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f3 = -1; ps(" f3"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f4[0] = -1; ps(" f4"); img(&s, sizeof s);
    putchar('\n');
}
static void t36(void) {
    union T36 s; ps("T36 "); pu(sizeof s); putchar(' '); pu(_Alignof(union T36));
    fill(&s, 0, sizeof s); s.f0 = -1; ps(" f0"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f1 = -1; ps(" f1"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); fill(&s.f2, 0xff, sizeof s.f2); ps(" f2"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f3 = -1; ps(" f3"); img(&s, sizeof s);
    fill(&s, 0x5a, sizeof s); s.f3 = -3LL; ps(" f3="); pi((long long)s.f3); img(&s, sizeof s);
    putchar('\n');
}
static void t37(void) {
    struct T37 s; ps("T37 "); pu(sizeof s); putchar(' '); pu(_Alignof(struct T37));
    fill(&s, 0, sizeof s); s.f0 = -1; ps(" f0"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f1 = -1; ps(" f1"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f2 = -1; ps(" f2"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f3 = -1; ps(" f3"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f4 = -1; ps(" f4"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f5 = -1; ps(" f5"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f7 = -1; ps(" f7"); img(&s, sizeof s);
    fill(&s, 0x5a, sizeof s); s.f0 = 3LL; ps(" f0="); pi((long long)s.f0); img(&s, sizeof s);
    fill(&s, 0x5a, sizeof s); s.f1 = 2132LL; ps(" f1="); pi((long long)s.f1); img(&s, sizeof s);
    fill(&s, 0x5a, sizeof s); s.f2 = 99LL; ps(" f2="); pi((long long)s.f2); img(&s, sizeof s);
    fill(&s, 0x5a, sizeof s); s.f3 = 0LL; ps(" f3="); pi((long long)s.f3); img(&s, sizeof s);
    fill(&s, 0x5a, sizeof s); s.f4 = 606LL; ps(" f4="); pi((long long)s.f4); img(&s, sizeof s);
    fill(&s, 0x5a, sizeof s); s.f5 = 7LL; ps(" f5="); pi((long long)s.f5); img(&s, sizeof s);
    fill(&s, 0x5a, sizeof s); s.f7 = 1LL; ps(" f7="); pi((long long)s.f7); img(&s, sizeof s);
    putchar('\n');
}
static void t38(void) {
    struct T38 s; ps("T38 "); pu(sizeof s); putchar(' '); pu(_Alignof(struct T38));
    fill(&s, 0, sizeof s); s.f0 = -1; ps(" f0"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f1 = -1; ps(" f1"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f2 = -1; ps(" f2"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f4 = -1; ps(" f4"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); fill(&s.f5, 0xff, sizeof s.f5); ps(" f5"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f6 = -1; ps(" f6"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f7 = -1; ps(" f7"); img(&s, sizeof s);
    fill(&s, 0x5a, sizeof s); s.f0 = 1LL; ps(" f0="); pi((long long)s.f0); img(&s, sizeof s);
    fill(&s, 0x5a, sizeof s); s.f1 = 0LL; ps(" f1="); pi((long long)s.f1); img(&s, sizeof s);
    fill(&s, 0x5a, sizeof s); s.f2 = 6LL; ps(" f2="); pi((long long)s.f2); img(&s, sizeof s);
    putchar('\n');
}
static void t39(void) {
    struct T39 s; ps("T39 "); pu(sizeof s); putchar(' '); pu(_Alignof(struct T39));
    fill(&s, 0, sizeof s); s.f0 = -1; ps(" f0"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f2 = -1; ps(" f2"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f4 = -1; ps(" f4"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f5 = -1; ps(" f5"); img(&s, sizeof s);
    fill(&s, 0x5a, sizeof s); s.f0 = 0LL; ps(" f0="); pi((long long)s.f0); img(&s, sizeof s);
    fill(&s, 0x5a, sizeof s); s.f2 = 0LL; ps(" f2="); pi((long long)s.f2); img(&s, sizeof s);
    fill(&s, 0x5a, sizeof s); s.f4 = 5760814LL; ps(" f4="); pi((long long)s.f4); img(&s, sizeof s);
    fill(&s, 0x5a, sizeof s); s.f5 = 1LL; ps(" f5="); pi((long long)s.f5); img(&s, sizeof s);
    putchar('\n');
}
static void t40(void) {
    struct T40 s; ps("T40 "); pu(sizeof s); putchar(' '); pu(_Alignof(struct T40));
    fill(&s, 0, sizeof s); s.f0 = -1; ps(" f0"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f1 = -1; ps(" f1"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f2 = -1; ps(" f2"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f3 = -1; ps(" f3"); img(&s, sizeof s);
    fill(&s, 0x5a, sizeof s); s.f0 = 50842LL; ps(" f0="); pi((long long)s.f0); img(&s, sizeof s);
    fill(&s, 0x5a, sizeof s); s.f3 = 0LL; ps(" f3="); pi((long long)s.f3); img(&s, sizeof s);
    putchar('\n');
}
static void t41(void) {
    struct T41 s; ps("T41 "); pu(sizeof s); putchar(' '); pu(_Alignof(struct T41));
    fill(&s, 0, sizeof s); s.f1 = -1; ps(" f1"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f2 = -1; ps(" f2"); img(&s, sizeof s);
    fill(&s, 0x5a, sizeof s); s.f1 = 0LL; ps(" f1="); pi((long long)s.f1); img(&s, sizeof s);
    fill(&s, 0x5a, sizeof s); s.f2 = -6LL; ps(" f2="); pi((long long)s.f2); img(&s, sizeof s);
    putchar('\n');
}
static void t42(void) {
    struct T42 s; ps("T42 "); pu(sizeof s); putchar(' '); pu(_Alignof(struct T42));
    fill(&s, 0, sizeof s); s.f1 = -1; ps(" f1"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f5 = -1; ps(" f5"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f6 = -1; ps(" f6"); img(&s, sizeof s);
    fill(&s, 0x5a, sizeof s); s.f1 = -15438282234078702LL; ps(" f1="); pi((long long)s.f1); img(&s, sizeof s);
    putchar('\n');
}
static void t43(void) {
    struct T43 s; ps("T43 "); pu(sizeof s); putchar(' '); pu(_Alignof(struct T43));
    fill(&s, 0, sizeof s); s.f0 = -1; ps(" f0"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f1 = -1; ps(" f1"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f2 = -1; ps(" f2"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f3 = -1; ps(" f3"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f4 = -1; ps(" f4"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f5 = -1; ps(" f5"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f6 = -1; ps(" f6"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f7 = -1; ps(" f7"); img(&s, sizeof s);
    fill(&s, 0x5a, sizeof s); s.f1 = 204LL; ps(" f1="); pi((long long)s.f1); img(&s, sizeof s);
    fill(&s, 0x5a, sizeof s); s.f2 = 1LL; ps(" f2="); pi((long long)s.f2); img(&s, sizeof s);
    fill(&s, 0x5a, sizeof s); s.f3 = -118LL; ps(" f3="); pi((long long)s.f3); img(&s, sizeof s);
    fill(&s, 0x5a, sizeof s); s.f4 = 203LL; ps(" f4="); pi((long long)s.f4); img(&s, sizeof s);
    fill(&s, 0x5a, sizeof s); s.f5 = 24LL; ps(" f5="); pi((long long)s.f5); img(&s, sizeof s);
    fill(&s, 0x5a, sizeof s); s.f7 = -16LL; ps(" f7="); pi((long long)s.f7); img(&s, sizeof s);
    putchar('\n');
}
static void t44(void) {
    struct T44 s; ps("T44 "); pu(sizeof s); putchar(' '); pu(_Alignof(struct T44));
    fill(&s, 0, sizeof s); s.f1 = -1; ps(" f1"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f2 = -1; ps(" f2"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f3 = -1; ps(" f3"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f4 = -1; ps(" f4"); img(&s, sizeof s);
    fill(&s, 0x5a, sizeof s); s.f2 = 12LL; ps(" f2="); pi((long long)s.f2); img(&s, sizeof s);
    fill(&s, 0x5a, sizeof s); s.f3 = 0LL; ps(" f3="); pi((long long)s.f3); img(&s, sizeof s);
    fill(&s, 0x5a, sizeof s); s.f4 = -2226LL; ps(" f4="); pi((long long)s.f4); img(&s, sizeof s);
    putchar('\n');
}
static void t45(void) {
    struct T45 s; ps("T45 "); pu(sizeof s); putchar(' '); pu(_Alignof(struct T45));
    fill(&s, 0, sizeof s); s.f0 = -1; ps(" f0"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f1 = -1; ps(" f1"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f2 = -1; ps(" f2"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f4 = -1; ps(" f4"); img(&s, sizeof s);
    fill(&s, 0x5a, sizeof s); s.f0 = 12LL; ps(" f0="); pi((long long)s.f0); img(&s, sizeof s);
    fill(&s, 0x5a, sizeof s); s.f1 = 7LL; ps(" f1="); pi((long long)s.f1); img(&s, sizeof s);
    putchar('\n');
}
static void t46(void) {
    struct T46 s; ps("T46 "); pu(sizeof s); putchar(' '); pu(_Alignof(struct T46));
    fill(&s, 0, sizeof s); s.f1 = -1; ps(" f1"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f2 = -1; ps(" f2"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f3 = -1; ps(" f3"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f5 = -1; ps(" f5"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f7 = -1; ps(" f7"); img(&s, sizeof s);
    fill(&s, 0x5a, sizeof s); s.f1 = 7LL; ps(" f1="); pi((long long)s.f1); img(&s, sizeof s);
    fill(&s, 0x5a, sizeof s); s.f2 = -4LL; ps(" f2="); pi((long long)s.f2); img(&s, sizeof s);
    fill(&s, 0x5a, sizeof s); s.f7 = -8423214411LL; ps(" f7="); pi((long long)s.f7); img(&s, sizeof s);
    putchar('\n');
}
static void t47(void) {
    struct T47 s; ps("T47 "); pu(sizeof s); putchar(' '); pu(_Alignof(struct T47));
    fill(&s, 0, sizeof s); s.f0 = -1; ps(" f0"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); fill(&s.f1, 0xff, sizeof s.f1); ps(" f1"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f2[0] = -1; ps(" f2"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); fill(&s.f3, 0xff, sizeof s.f3); ps(" f3"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f4 = -1; ps(" f4"); img(&s, sizeof s);
    fill(&s, 0x5a, sizeof s); s.f0 = 54LL; ps(" f0="); pi((long long)s.f0); img(&s, sizeof s);
    fill(&s, 0x5a, sizeof s); s.f4 = 23064LL; ps(" f4="); pi((long long)s.f4); img(&s, sizeof s);
    putchar('\n');
}
static void t48(void) {
    struct T48 s; ps("T48 "); pu(sizeof s); putchar(' '); pu(_Alignof(struct T48));
    fill(&s, 0, sizeof s); s.f0 = -1; ps(" f0"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); fill(&s.f1, 0xff, sizeof s.f1); ps(" f1"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f2 = -1; ps(" f2"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f3 = -1; ps(" f3"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f5 = -1; ps(" f5"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f6 = -1; ps(" f6"); img(&s, sizeof s);
    fill(&s, 0x5a, sizeof s); s.f2 = -30LL; ps(" f2="); pi((long long)s.f2); img(&s, sizeof s);
    fill(&s, 0x5a, sizeof s); s.f3 = 13LL; ps(" f3="); pi((long long)s.f3); img(&s, sizeof s);
    fill(&s, 0x5a, sizeof s); s.f6 = 9LL; ps(" f6="); pi((long long)s.f6); img(&s, sizeof s);
    putchar('\n');
}
static void t49(void) {
    union T49 s; ps("T49 "); pu(sizeof s); putchar(' '); pu(_Alignof(union T49));
    fill(&s, 0, sizeof s); s.f2[0] = -1; ps(" f2"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f3 = -1; ps(" f3"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f4[0] = -1; ps(" f4"); img(&s, sizeof s);
    putchar('\n');
}
static void t50(void) {
    struct T50 s; ps("T50 "); pu(sizeof s); putchar(' '); pu(_Alignof(struct T50));
    fill(&s, 0, sizeof s); s.f1 = -1; ps(" f1"); img(&s, sizeof s);
    fill(&s, 0x5a, sizeof s); s.f1 = 1LL; ps(" f1="); pi((long long)s.f1); img(&s, sizeof s);
    putchar('\n');
}
static void t51(void) {
    struct T51 s; ps("T51 "); pu(sizeof s); putchar(' '); pu(_Alignof(struct T51));
    fill(&s, 0, sizeof s); s.f0 = -1; ps(" f0"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f1 = -1; ps(" f1"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f2 = -1; ps(" f2"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f3 = -1; ps(" f3"); img(&s, sizeof s);
    fill(&s, 0x5a, sizeof s); s.f1 = 1LL; ps(" f1="); pi((long long)s.f1); img(&s, sizeof s);
    fill(&s, 0x5a, sizeof s); s.f3 = 3LL; ps(" f3="); pi((long long)s.f3); img(&s, sizeof s);
    putchar('\n');
}
static void t52(void) {
    struct T52 s; ps("T52 "); pu(sizeof s); putchar(' '); pu(_Alignof(struct T52));
    fill(&s, 0, sizeof s); s.f0 = -1; ps(" f0"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f1 = -1; ps(" f1"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f2[0] = -1; ps(" f2"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f3 = -1; ps(" f3"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f4 = -1; ps(" f4"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f5 = -1; ps(" f5"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f6 = -1; ps(" f6"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f7 = -1; ps(" f7"); img(&s, sizeof s);
    fill(&s, 0x5a, sizeof s); s.f3 = 29LL; ps(" f3="); pi((long long)s.f3); img(&s, sizeof s);
    fill(&s, 0x5a, sizeof s); s.f4 = -5LL; ps(" f4="); pi((long long)s.f4); img(&s, sizeof s);
    fill(&s, 0x5a, sizeof s); s.f5 = 2262474676531LL; ps(" f5="); pi((long long)s.f5); img(&s, sizeof s);
    fill(&s, 0x5a, sizeof s); s.f6 = 0LL; ps(" f6="); pi((long long)s.f6); img(&s, sizeof s);
    fill(&s, 0x5a, sizeof s); s.f7 = 519215LL; ps(" f7="); pi((long long)s.f7); img(&s, sizeof s);
    putchar('\n');
}
static void t53(void) {
    struct T53 s; ps("T53 "); pu(sizeof s); putchar(' '); pu(_Alignof(struct T53));
    fill(&s, 0, sizeof s); s.f0 = -1; ps(" f0"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f1[0] = -1; ps(" f1"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f2 = -1; ps(" f2"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f5 = -1; ps(" f5"); img(&s, sizeof s);
    fill(&s, 0x5a, sizeof s); s.f0 = 6LL; ps(" f0="); pi((long long)s.f0); img(&s, sizeof s);
    putchar('\n');
}
static void t54(void) {
    struct T54 s; ps("T54 "); pu(sizeof s); putchar(' '); pu(_Alignof(struct T54));
    fill(&s, 0, sizeof s); s.f0 = -1; ps(" f0"); img(&s, sizeof s);
    fill(&s, 0x5a, sizeof s); s.f0 = 1LL; ps(" f0="); pi((long long)s.f0); img(&s, sizeof s);
    putchar('\n');
}
static void t55(void) {
    struct T55 s; ps("T55 "); pu(sizeof s); putchar(' '); pu(_Alignof(struct T55));
    fill(&s, 0, sizeof s); s.f0 = -1; ps(" f0"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f1 = -1; ps(" f1"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f2 = -1; ps(" f2"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f3 = -1; ps(" f3"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f5 = -1; ps(" f5"); img(&s, sizeof s);
    fill(&s, 0x5a, sizeof s); s.f1 = 1LL; ps(" f1="); pi((long long)s.f1); img(&s, sizeof s);
    fill(&s, 0x5a, sizeof s); s.f2 = 62LL; ps(" f2="); pi((long long)s.f2); img(&s, sizeof s);
    fill(&s, 0x5a, sizeof s); s.f5 = 2687LL; ps(" f5="); pi((long long)s.f5); img(&s, sizeof s);
    putchar('\n');
}
static void t56(void) {
    struct T56 s; ps("T56 "); pu(sizeof s); putchar(' '); pu(_Alignof(struct T56));
    fill(&s, 0, sizeof s); s.f0 = -1; ps(" f0"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f1 = -1; ps(" f1"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); fill(&s.f2, 0xff, sizeof s.f2); ps(" f2"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f3 = -1; ps(" f3"); img(&s, sizeof s);
    fill(&s, 0x5a, sizeof s); s.f0 = 548485722124892037LL; ps(" f0="); pi((long long)s.f0); img(&s, sizeof s);
    fill(&s, 0x5a, sizeof s); s.f1 = 257221072934255LL; ps(" f1="); pi((long long)s.f1); img(&s, sizeof s);
    putchar('\n');
}
static void t57(void) {
    struct T57 s; ps("T57 "); pu(sizeof s); putchar(' '); pu(_Alignof(struct T57));
    fill(&s, 0, sizeof s); s.f0 = -1; ps(" f0"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f2 = -1; ps(" f2"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f3 = -1; ps(" f3"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f4 = -1; ps(" f4"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f5 = -1; ps(" f5"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f6 = -1; ps(" f6"); img(&s, sizeof s);
    fill(&s, 0x5a, sizeof s); s.f0 = 23LL; ps(" f0="); pi((long long)s.f0); img(&s, sizeof s);
    fill(&s, 0x5a, sizeof s); s.f4 = 7LL; ps(" f4="); pi((long long)s.f4); img(&s, sizeof s);
    fill(&s, 0x5a, sizeof s); s.f5 = 75LL; ps(" f5="); pi((long long)s.f5); img(&s, sizeof s);
    putchar('\n');
}
static void t58(void) {
    struct T58 s; ps("T58 "); pu(sizeof s); putchar(' '); pu(_Alignof(struct T58));
    fill(&s, 0, sizeof s); s.f0 = -1; ps(" f0"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f1 = -1; ps(" f1"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f2 = -1; ps(" f2"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f3 = -1; ps(" f3"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f4 = -1; ps(" f4"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f5 = -1; ps(" f5"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f6 = -1; ps(" f6"); img(&s, sizeof s);
    fill(&s, 0x5a, sizeof s); s.f1 = 1755LL; ps(" f1="); pi((long long)s.f1); img(&s, sizeof s);
    fill(&s, 0x5a, sizeof s); s.f2 = 1LL; ps(" f2="); pi((long long)s.f2); img(&s, sizeof s);
    fill(&s, 0x5a, sizeof s); s.f3 = 588887LL; ps(" f3="); pi((long long)s.f3); img(&s, sizeof s);
    fill(&s, 0x5a, sizeof s); s.f6 = -1LL; ps(" f6="); pi((long long)s.f6); img(&s, sizeof s);
    putchar('\n');
}
static void t59(void) {
    struct T59 s; ps("T59 "); pu(sizeof s); putchar(' '); pu(_Alignof(struct T59));
    fill(&s, 0, sizeof s); s.f0 = -1; ps(" f0"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f1 = -1; ps(" f1"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f2 = -1; ps(" f2"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f3 = -1; ps(" f3"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f4 = -1; ps(" f4"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f5 = -1; ps(" f5"); img(&s, sizeof s);
    fill(&s, 0, sizeof s); s.f6 = -1; ps(" f6"); img(&s, sizeof s);
    fill(&s, 0x5a, sizeof s); s.f0 = 2433LL; ps(" f0="); pi((long long)s.f0); img(&s, sizeof s);
    fill(&s, 0x5a, sizeof s); s.f1 = 0LL; ps(" f1="); pi((long long)s.f1); img(&s, sizeof s);
    fill(&s, 0x5a, sizeof s); s.f2 = 0LL; ps(" f2="); pi((long long)s.f2); img(&s, sizeof s);
    fill(&s, 0x5a, sizeof s); s.f3 = -132872LL; ps(" f3="); pi((long long)s.f3); img(&s, sizeof s);
    fill(&s, 0x5a, sizeof s); s.f4 = 0LL; ps(" f4="); pi((long long)s.f4); img(&s, sizeof s);
    fill(&s, 0x5a, sizeof s); s.f6 = -142LL; ps(" f6="); pi((long long)s.f6); img(&s, sizeof s);
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
