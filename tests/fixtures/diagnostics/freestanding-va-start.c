#include <stdarg.h>
void fixed(int n) { va_list ap; va_start(ap, n); va_end(ap); }
