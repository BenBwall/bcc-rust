/* The buffer of __builtin_setjmp and __builtin_longjmp: GCC's void *, which
   any object pointer converts to, and Clang's void **, which takes only a
   pointer to void * and refuses the rest as an argument of that type. */
struct S { int x; };
void fn(void);
void *buffer[5];
void **pointer;
void *const constant[5] = {0};
const void **nested;
char bytes[64];
volatile char volatile_bytes[64];
const int *read_only;
struct S *record;
long words[5];
int **ints;
double real;
int f(void) {
  __builtin_longjmp(buffer, 1);
  __builtin_longjmp(pointer, 1);
  __builtin_longjmp(0, 1);
  __builtin_longjmp(constant, 1);
  __builtin_longjmp(nested, 1);
  __builtin_longjmp(bytes, 1);
  __builtin_longjmp(volatile_bytes, 1);
  __builtin_longjmp(read_only, 1);
  __builtin_longjmp(record, 1);
  __builtin_longjmp(words, 1);
  __builtin_longjmp(ints, 1);
  __builtin_longjmp(fn, 1);
  __builtin_longjmp(5, 1);
  return __builtin_setjmp(bytes) + __builtin_setjmp(&fn) + __builtin_setjmp(buffer);
}
