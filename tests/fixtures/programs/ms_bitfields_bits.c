/* Bit patterns, sizes and alignments of records with bit-fields, which
   tests/integration/ms_bitfields.rs compares with what GCC 14, Clang 23 and
   TCC print under -mms-bitfields. */
#include <stdio.h>
#include <string.h>
#include <stddef.h>
static void dump(const char *name, void *p, size_t size, size_t align) {
    printf("%s size %zu align %zu bits ", name, size, align);
    for (size_t i = size; i--;)
        printf("%02x", ((unsigned char *)p)[i]);
    printf("\n");
}
#define SHOW(T, ...) do { T v; memset(&v, 0, sizeof v); __VA_ARGS__; dump(#T, &v, sizeof v, _Alignof(T)); } while (0)
struct A { char a:4; int b:8; char c; };
struct B { int a:3; long long b:40; short c:5; };
struct C { char a; int :0; char b; };
struct D { char a:2; int :0; char b:3; };
struct E { short a:9; short b:9; short c:9; };
union U { char a:3; int b:20; };
struct F { int a:31; int b:2; };
struct G { long long a:1; char b:7; long long c:1; };
struct __attribute__((packed)) P { char a:4; int b:8; char c; };
#pragma pack(push, 2)
struct Q { char a:4; int b:20; char c:2; };
#pragma pack(pop)
struct __attribute__((gcc_struct)) H { char a:4; int b:8; char c; };
struct __attribute__((ms_struct)) I { char a:4; int b:8; char c; };
struct J { char a:3; short b:3; char c; long long d:5; };
int main(void) {
    SHOW(struct A, v.a = -1, v.b = -1, v.c = -1);
    SHOW(struct B, v.a = -1, v.b = -1, v.c = -1);
    SHOW(struct C, v.a = -1, v.b = -1);
    SHOW(struct D, v.a = -1, v.b = -1);
    SHOW(struct E, v.a = -1, v.b = -1, v.c = -1);
    SHOW(union U, v.b = -1);
    SHOW(struct F, v.a = -1, v.b = -1);
    SHOW(struct G, v.a = -1, v.b = -1, v.c = -1);
    SHOW(struct P, v.a = -1, v.b = -1, v.c = -1);
    SHOW(struct Q, v.a = -1, v.b = -1, v.c = -1);
    SHOW(struct H, v.a = -1, v.b = -1, v.c = -1);
    SHOW(struct I, v.a = -1, v.b = -1, v.c = -1);
    SHOW(struct J, v.a = -1, v.b = -1, v.c = -1, v.d = -1);
    printf("offsets %zu %zu %zu %zu\n", offsetof(struct A, c), offsetof(struct C, b), offsetof(struct I, c), offsetof(struct J, c));
    return 0;
}
