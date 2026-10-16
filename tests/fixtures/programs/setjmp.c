/* setjmp and longjmp across frames, nested buffers, repeated returns, a
   protected-call chain shaped like Lua's ldo.c, and glibc's _setjmp and
   sigsetjmp. Each check returns its own nonzero status on failure.
   C99 7.13: every setjmp appears in a context 7.13.1.1p4 allows, and every
   automatic object changed between a setjmp and its longjmp is volatile
   (7.13.2.1p3).

   -DMINGW_DECLARATIONS replaces <setjmp.h> with MinGW-w64's declarations
   and its non-Clang setjmp macro, which passes mingw_getsp() to _setjmp. */
#ifdef MINGW_DECLARATIONS
typedef unsigned long long jmp_buf[32];
void *mingw_getsp(void);
int __attribute__((__nothrow__, __returns_twice__)) _setjmp(jmp_buf, void *);
__attribute__((__noreturn__)) void longjmp(jmp_buf, int);
#define setjmp(BUF) _setjmp((BUF), mingw_getsp())
#else
#include <setjmp.h>
#endif

static jmp_buf outer, inner;
static int global;

static int depth3(int v) {
  global += 1;
  longjmp(inner, v);
}
static int depth2(int v) { return depth3(v) + 1; }
static int depth1(int v) { return depth2(v) + 1; }

/* A longjmp from three frames down, with 0 delivered as 1, then a jump to
   the outer buffer from the inner handler. */
static int nested(void) {
  volatile int stage = 0;
  switch (setjmp(outer)) {
  case 0:
    stage = 1;
    if (setjmp(inner) == 0) {
      stage = 2;
      depth1(0);
      return 10;
    }
    if (stage != 2 || global != 1)
      return 11;
    stage = 3;
    longjmp(outer, 7);
  case 7:
    return stage == 3 ? 0 : 12;
  default:
    return 13;
  }
}

/* One setjmp returning three times, the last value chosen by longjmp. */
static int repeated(void) {
  jmp_buf here;
  volatile int count = 0;
  switch (setjmp(here)) {
  case 0:
    if (count != 0)
      return 20;
    count = 1;
    longjmp(here, 0);
  case 1:
    if (count != 1)
      return 21;
    count = 2;
    longjmp(here, 2);
  case 2:
    if (count != 2)
      return 22;
    count = 3;
    break;
  default:
    return 23;
  }
  return count == 3 ? 0 : 24;
}

static void thrower(jmp_buf *buffer) { longjmp(*buffer, 1); }

/* Objects not changed since the setjmp keep their values in registers or
   memory: `base` and the loop counter `i` are not volatile. */
static int unchanged(int n) {
  jmp_buf buffer;
  int base = n * 3;
  volatile int seen = 0;
  for (int i = 0; i < n; i++) {
    if (setjmp(buffer) == 0) {
      thrower(&buffer);
      return 30;
    }
    seen += base + i;
  }
  return seen == 54 ? 0 : 31;
}

/* Lua's error handling: a chain of handlers, each with its buffer inside
   an automatic structure whose address the callee reaches. */
struct handler {
  struct handler *previous;
  jmp_buf buffer;
  volatile int status;
};
static struct handler *current;

static void raise_error(int status) {
  current->status = status;
  longjmp(current->buffer, 1);
}
static int protected_call(void (*f)(int), int argument) {
  struct handler h;
  h.status = 0;
  h.previous = current;
  current = &h;
  if (setjmp(h.buffer) == 0)
    f(argument);
  current = h.previous;
  return h.status;
}
static void fails(int status) {
  if (status)
    raise_error(status);
}
static void nests(int status) {
  if (protected_call(fails, status) != status)
    raise_error(99);
  if (protected_call(fails, 0) != 0)
    raise_error(98);
  raise_error(status + 1);
}

static int protected_calls(void) {
  if (protected_call(fails, 0) != 0)
    return 40;
  if (protected_call(nests, 5) != 6)
    return 41;
  return current == 0 ? 0 : 42;
}

#ifdef __GLIBC__
/* glibc's _setjmp/_longjmp (Lua's POSIX path) and sigsetjmp, which
   expands to __sigsetjmp. */
static int posix(void) {
  sigjmp_buf signals;
  jmp_buf plain;
  volatile int n = 0;
  if (sigsetjmp(signals, 1) == 0) {
    n = 1;
    siglongjmp(signals, 3);
  }
  if (n != 1)
    return 50;
  if (_setjmp(plain) == 0) {
    n = 2;
    _longjmp(plain, 4);
  }
  return n == 2 ? 0 : 51;
}
#else
static int posix(void) { return 0; }
#endif

int main(void) {
  int status;
  if ((status = nested()) || (status = repeated()) || (status = unchanged(4))
      || (status = protected_calls()) || (status = posix()))
    return status;
  return global == 1 ? 0 : 60;
}
