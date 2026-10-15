/* bcc ignores every form of #pragma pack: all sizes below should differ from natural layout.
   Clang prints "A5 B5 C6 D5 E12 F5 G8"; bcc prints "A8 B8 C8 D5 E16 F8 G8". */
#include <stdio.h>
#pragma pack(1)
struct A { char c; int i; };
#pragma pack()
#pragma pack(push, 1)
struct B { char c; int i; };
#pragma pack(pop)
#pragma pack(push)
#pragma pack(2)
struct C { char c; int i; };
#pragma pack(pop)
struct D { char c; int i; } __attribute__((packed));
#pragma pack(push, 1)
#pragma pack(push, 4)
struct E { char c; long long i; };
#pragma pack(pop)
struct F { char c; int i; };
#pragma pack(pop)
struct G { char c; int i; };
int main(void) {
    printf("A%d B%d C%d D%d E%d F%d G%d\n", (int)sizeof(struct A), (int)sizeof(struct B), (int)sizeof(struct C),
           (int)sizeof(struct D), (int)sizeof(struct E), (int)sizeof(struct F), (int)sizeof(struct G));
    return 0;
}
