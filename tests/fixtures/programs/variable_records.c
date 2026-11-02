/* GNU structures and unions with members of variable size, laid out at run
   time as GCC lays them out: sizeof and offsetof (with run-time indices),
   members after one of variable size (including bit-fields, which GCC
   packs bit by bit there, and members that need more alignment than the
   run-time base has), copies, arrays of such records and pointer
   arithmetic on them, records nested in records, unions, bounds saved at
   the definition, a definition in a loop, packed records, a statement
   expression's value of variable size, records passed through `...`
   by invisible reference, and members after one whose `aligned` attribute
   or packing does not round its variable size (the next member is aligned
   at run time by its own alignment), and bit-fields after one with explicit
   alignment, which still rounds them up (at run time if need be; capped by
   `#pragma pack` but not by packing), unnamed, zero-width (aligned to the
   declared type, a typedef's alignment included) and several in a row.
   Prints what GCC prints. */
#include <stdarg.h>
#include <stddef.h>
#include <stdio.h>
#include <string.h>
static unsigned sum(const void *p, size_t n) {
  const unsigned char *c = p; unsigned s = 0;
  for (size_t i = 0; i < n; i++) s = s * 31 + c[i];
  return s;
}
static void take(int n, ...) {
  struct V { short h; int v[n]; char t; };
  va_list ap; va_start(ap, n);
  struct V a, b;
  a = va_arg(ap, struct V);
  int after = va_arg(ap, int);
  b = va_arg(ap, struct V);
  va_end(ap);
  printf("va %d %d %d %d %d %d\n", a.h, a.v[n - 1], a.t, after, b.v[0], b.t);
}
static int run(int n, int m) {
  struct S { char c; int a[n]; short s; double d[m]; char e; };
  printf("S %zu %zu %zu %zu %zu\n", sizeof(struct S), offsetof(struct S, a),
         offsetof(struct S, s), offsetof(struct S, d), offsetof(struct S, e));
  printf("idx %zu %zu\n", offsetof(struct S, a[n - 1]), offsetof(struct S, d[m / 2]));
  struct S x, y, arr[3];
  memset(&x, 0, sizeof x);
  x.c = 1; x.s = 2; x.e = 3;
  for (int i = 0; i < n; i++) x.a[i] = i * 7;
  for (int i = 0; i < m; i++) x.d[i] = i / 4.0;
  y = x;
  arr[2] = y;
  struct S *p = arr;
  p += 2;
  printf("copy %d %d %d %d %g %u\n", p->c, p->s, p->e, p->a[n - 1], p->d[m - 1], sum(p, sizeof *p));
  printf("diff %td %zu\n", &arr[2] - &arr[0], (size_t)((char *)&arr[1] - (char *)&arr[0]));
  struct O { char z; struct S in; char y; struct S two[2]; long w; } o;
  memset(&o, 0, sizeof o);
  o.in = x; o.two[1] = x; o.y = 9; o.w = 77;
  printf("O %zu %zu %zu %zu %d %d %ld\n", sizeof o, offsetof(struct O, y), offsetof(struct O, two),
         offsetof(struct O, w), o.two[1].a[n - 1], o.y, o.w);
  union U { char a[n]; int i; double d[m]; } u;
  printf("U %zu %zu\n", sizeof u, _Alignof(union U));
  int k = n;
  struct T { int v[k]; } t;
  k = 100;
  printf("T %zu %zu\n", sizeof t, sizeof(struct T));
  typedef struct { char b[n]; int i; } TD;
  TD td[2];
  printf("TD %zu %zu\n", sizeof td, offsetof(TD, i));
  struct V { short h; int v[n]; char t; } v1, v2;
  v1.h = 5; v1.t = 6; v2.t = 8;
  for (int i = 0; i < n; i++) { v1.v[i] = 10 + i; v2.v[i] = 20 + i; }
  take(n, v1, 99, v2);
  int total = 0;
  for (int r = 1; r <= 3; r++) {
    struct L { int q[r]; char c; } l;
    l.c = (char)r;
    l.q[r - 1] = r * 100;
    total += (int)sizeof l + l.q[r - 1] + l.c;
  }
  printf("loop %d\n", total);
  struct P { unsigned char a[n]; unsigned b : 3, c : 5; unsigned d : 7; } __attribute__((packed)) pk;
  memset(&pk, 0, sizeof pk);
  pk.b = 5; pk.c = 17; pk.d = 99;
  printf("P %zu %u %u %u %u\n", sizeof pk, pk.b, pk.c, pk.d, sum(&pk, sizeof pk));
  return (int)sizeof(struct S);
}
static int value(int n) {
  struct W { char c; int v[n]; } a;
  for (int i = 0; i < n; i++)
    a.v[i] = i + 1;
  struct W b;
  b = ({ struct W t; t = a; t.v[n - 1] *= 10; t; });
  return b.v[n - 1] + b.v[0];
}
static void aligned(int n) {
  struct A1 { char a[n] __attribute__((aligned(32))); long c; } a1;
  struct A2 { char a[n] __attribute__((aligned(2))); short c; } a2;
  struct A3 { struct { char x[n]; } a __attribute__((aligned(16))); long double c; } a3;
  struct A4 { char a[n] __attribute__((aligned(8))); int b : 3; char c; } a4;
  struct A5 { char a[n] __attribute__((aligned(16))); struct { int x[n]; char y; } d; char c; } a5;
  struct A6 { char a[n]; int : 0; char c; } __attribute__((packed)) a6;
#pragma pack(push, 2)
  struct A7 { char a[n] __attribute__((aligned(8))); long c; } a7;
#pragma pack(pop)
  memset(&a4, 0, sizeof a4);
  a1.c = 11; a2.c = 12; a3.c = 13; a4.b = -2; a4.c = 14; a5.c = 15; a6.c = 16; a7.c = 17;
  a5.d.x[n - 1] = 55; a5.d.y = 5;
  printf("A1 %zu %zu %ld\n", sizeof a1, offsetof(struct A1, c), *(long *)((char *)&a1 + offsetof(struct A1, c)));
  printf("A2 %zu %zu %d\n", sizeof a2, offsetof(struct A2, c), a2.c);
  printf("A3 %zu %zu %zu %g\n", sizeof a3, _Alignof(struct A3), offsetof(struct A3, c), (double)a3.c);
  printf("A4 %zu %zu %d %d %u\n", sizeof a4, offsetof(struct A4, c), a4.b, a4.c,
         sum((char *)&a4 + n, sizeof a4 - n));
  printf("A5 %zu %zu %zu %d %d %d\n", sizeof a5, offsetof(struct A5, d), offsetof(struct A5, c),
         a5.d.x[n - 1], a5.d.y, a5.c);
  printf("A6 %zu %zu %d\n", sizeof a6, offsetof(struct A6, c), a6.c);
  printf("A7 %zu %zu %zu %ld\n", sizeof a7, _Alignof(struct A7), offsetof(struct A7, c), a7.c);
}
typedef int A16 __attribute__((aligned(16)));
typedef long long L2 __attribute__((aligned(2)));
#define SHOW(tag, x, format, ...)                                                \
  printf(#tag " %zu %zu %zu %u " format "\n", sizeof x, _Alignof(struct tag),     \
         offsetof(struct tag, c), sum(&x, sizeof x), __VA_ARGS__)
static void bitfields(int n) {
  struct B1 { char a[n]; int b : 3 __attribute__((aligned(8))); char c; } b1;
  struct B2 { char a[n]; int z : 3; int b : 9 __attribute__((aligned(2))); char c; } b2;
  struct B3 { char a[n]; int : 3 __attribute__((aligned(16))); char c; } b3;
  struct B4 {
    char a[n];
    int b : 3 __attribute__((aligned(4)));
    int d : 5 __attribute__((aligned(2)));
    short z : 4 __attribute__((aligned(8)));
    char c;
  } b4;
  struct B5 { char a[n]; int b : 3 __attribute__((packed, aligned(8))); char c; } __attribute__((packed)) b5;
  struct B6 { char a[n]; A16 b : 5 __attribute__((aligned(4))); char c; } b6;
  struct B7 { char a[n]; A16 : 0; char c; } b7;
  struct B8 { char a[n]; char z; L2 : 0; char c; } b8;
#pragma pack(push, 2)
  struct B9 { char a[n]; int z : 3; long b : 7 __attribute__((aligned(32))); char c; } b9;
#pragma pack(pop)
#pragma pack(push, 4)
  struct B10 { char a[n]; int b : 3; char c; } __attribute__((packed)) b10;
#pragma pack(pop)
  union B11 { char a[n]; char z; L2 b : 33; } b11;
  memset(&b1, 0, sizeof b1); memset(&b2, 0, sizeof b2); memset(&b3, 0, sizeof b3);
  memset(&b4, 0, sizeof b4); memset(&b5, 0, sizeof b5); memset(&b6, 0, sizeof b6);
  memset(&b7, 0, sizeof b7); memset(&b8, 0, sizeof b8); memset(&b9, 0, sizeof b9);
  memset(&b10, 0, sizeof b10);
  b1.b = 3; b1.c = 1; b2.z = -1; b2.b = 200; b2.c = 2; b3.c = 3;
  b4.b = 2; b4.d = -7; b4.z = 5; b4.c = 4; b5.b = -3; b5.c = 5; b6.b = 9; b6.c = 6;
  b7.c = 7; b8.z = 1; b8.c = 8; b9.z = 2; b9.b = -60; b9.c = 9; b10.b = 1; b10.c = 10;
  SHOW(B1, b1, "%d %d", b1.b, b1.c);
  SHOW(B2, b2, "%d %d %d", b2.z, b2.b, b2.c);
  SHOW(B3, b3, "%d", b3.c);
  SHOW(B4, b4, "%d %d %d %d", b4.b, b4.d, b4.z, b4.c);
  SHOW(B5, b5, "%d %d", b5.b, b5.c);
  SHOW(B6, b6, "%d %d", b6.b, b6.c);
  SHOW(B7, b7, "%d", b7.c);
  SHOW(B8, b8, "%d", b8.c);
  SHOW(B9, b9, "%d %d %d", b9.z, b9.b, b9.c);
  SHOW(B10, b10, "%d %d", b10.b, b10.c);
  printf("B11 %zu %zu\n", sizeof b11, _Alignof(union B11));
}
int main(void) {
  printf("value %d %d\n", value(1), value(4));
  printf("%d\n", run(3, 2));
  printf("%d\n", run(1, 5));
  printf("%d\n", run(8, 1));
  aligned(1);
  aligned(3);
  aligned(6);
  bitfields(1);
  bitfields(3);
  bitfields(6);
  return 0;
}
