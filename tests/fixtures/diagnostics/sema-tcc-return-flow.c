/* TCC sees the C library without its attributes, keeps the code after a
   statement expression, and compiles a deferred inline function after the
   unit, naming the line before its closing brace. */
#include "sema-tcc-return-flow.h"
typedef long jmp_buf[8];
void exit(int);
void abort(void);
void longjmp(jmp_buf, int);
_Noreturn void quit(void);
__attribute__((noreturn)) void stop(void);
jmp_buf buffer;
int calls_exit(void) { exit(1); }
int calls_abort(void) { abort(); }
int calls_longjmp(void) { longjmp(buffer, 1); }
int calls_quit(void) { quit(); }
int calls_stop(void) { stop(); }
int inner_return(int x) { ({ return x; }); }
inline int plain(int x) { if (x) return 1; }
static inline int unused(int x) { if (x) return 1; }
static inline int called(int x)
{
  if (x)
    return 1;
}
static inline int chained(int x) { return x ? called(x) : 0; }
static inline int caller(int x) {
  if (x) return chained(x);
  }
int declared(void);
inline int declared(void) { }
extern inline int external(void) { }
int main(void) { return plain(1) + caller(1) + header_inline(1); }
int late(int x) { if (x) return 1; }
