/* GNU structures and unions with members of variable size, laid out at run
   time as GCC lays them out: sizeof and offsetof (with run-time indices),
   members after one of variable size (including bit-fields, which GCC
   packs bit by bit there, and members that need more alignment than the
   run-time base has), copies, arrays of such records and pointer
   arithmetic on them, records nested in records, unions, bounds saved at
   the definition, a definition in a loop, packed records, a statement
   expression's value of variable size, and records passed through `...`
   by invisible reference. Prints what GCC prints. */
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
int main(void) {
  printf("value %d %d\n", value(1), value(4));
  printf("%d\n", run(3, 2));
  printf("%d\n", run(1, 5));
  printf("%d\n", run(8, 1));
  return 0;
}
