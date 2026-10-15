/* glibc and MinGW-w64 define INFINITY (and NAN) with __builtin_inff()/__builtin_nanf(): bcc reports
   "construct not supported by code generation: its type was not analyzed" (SQLite, many math programs). */
#include <math.h>
double f(void) { return INFINITY; }
int main(void) { return f() > 1e308 ? 0 : 1; }
