/* Shared semantic/Clang LP64 probe. Assertions are a C11 inspection aid;
   the operands exercise C99 expression, layout and initializer rules. */
#define CHECK(c) _Static_assert(c, #c)
struct S { char c; int a[3]; long n; };
struct S s;
int inferred[] = {[4]=1, 2};
int matrix[][3] = {1,2,3,4,5,6};
char text[] = "abc";
int f(int);
CHECK(sizeof s == 24);
CHECK(sizeof s.a == 12);
CHECK(sizeof s.a[0] == 4);
CHECK(sizeof &s == 8);
CHECK(sizeof &f == 8);
CHECK(sizeof (1 + 2UL) == 8);
CHECK(sizeof ((short)1 + (unsigned short)2) == 4);
CHECK(sizeof (1.0f + 1) == 4);
CHECK(sizeof (1.0L + 1.0) == 16);
CHECK(sizeof ((float _Complex)1 + 1.0) == 16);
CHECK(sizeof (1 ? (int *)0 : (void *)0) == 8);
CHECK(sizeof (1, 2L) == 8);
CHECK(sizeof inferred == 24);
CHECK(sizeof matrix == 24);
CHECK(sizeof text == 4);
CHECK(sizeof "abc" == 4);
CHECK(sizeof L"abc" == 16);
CHECK(sizeof ((int[]){1,2,3}) == 12);
CHECK(sizeof (sizeof s) == 8);
