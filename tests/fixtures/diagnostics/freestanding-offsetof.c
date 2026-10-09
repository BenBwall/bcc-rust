#include <stddef.h>
struct S { int bit:3; int array[2]; };
int a = offsetof(struct S, bit);
int b = offsetof(struct S, absent);
int c = offsetof(int, field);
int d = offsetof(struct S, array.field);
