// Sizes, ranges, alignments and layouts on x86-64.
#include <stdio.h>
#include <stddef.h>
#include <float.h>
#include <limits.h>
struct point { int x; int y; };
struct entry { char tag; int count; };
int main(void) {
    printf("char %zu short %zu int %zu long %zu llong %zu ptr %zu size_t %zu\n",
           sizeof(char), sizeof(short), sizeof(int), sizeof(long), sizeof(long long),
           sizeof(int *), sizeof(size_t));
    printf("float %zu double %zu ldouble %zu align ldouble %zu\n", sizeof(float), sizeof(double),
           sizeof(long double), _Alignof(long double));
    printf("FLT_DIG %d DBL_DIG %d LDBL_DIG %d LDBL_MANT_DIG %d\n", FLT_DIG, DBL_DIG, LDBL_DIG, LDBL_MANT_DIG);
    printf("CHAR_MIN %d CHAR_MAX %d INT_MIN %d INT_MAX %d UINT_MAX %u\n", CHAR_MIN, CHAR_MAX, INT_MIN, INT_MAX, UINT_MAX);
    printf("struct point %zu align %zu x@%zu y@%zu\n", sizeof(struct point), _Alignof(struct point),
           offsetof(struct point, x), offsetof(struct point, y));
    printf("struct entry %zu align %zu tag@%zu count@%zu\n", sizeof(struct entry), _Alignof(struct entry),
           offsetof(struct entry, tag), offsetof(struct entry, count));
    printf("align int %zu ptr %zu\n", _Alignof(int), _Alignof(int *));
    int values[3];
    printf("sizeof values %zu, 'A' %d, sizeof 'A' %zu\n", sizeof values, 'A', sizeof 'A');
    return 0;
}
