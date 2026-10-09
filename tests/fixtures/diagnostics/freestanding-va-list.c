#include <stdarg.h>
void f(int n, ...) { va_list ap; va_start(1, n); va_end(1); va_copy(ap, 1); va_arg(1, int); }
