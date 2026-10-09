#include <stdarg.h>
struct incomplete;
void f(int n, ...) { va_list ap; va_arg(ap, void); va_arg(ap, struct incomplete); va_arg(ap, int[2]); }
