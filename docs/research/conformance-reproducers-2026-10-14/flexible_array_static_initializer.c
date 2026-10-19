/* GNU extension: a static object with a flexible array member initialized
   by a brace list. bcc: "excess initializer for the current object".
   Clang and GCC print "3 3 12". */
#include <stdio.h>
struct F { int n; int v[]; };
static struct F f = { 3, { 1, 2, 3 } };
static const struct { short len; char text[]; } msg = { 5, "hello" };
int main(void) { printf("%d %d %d\n", f.n, f.v[2], msg.len + (int)sizeof("hello") + 1); return 0; }
