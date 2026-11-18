/* Clang's nonnull parameters and returns_nonnull calls, compared with a null
   pointer or converted to _Bool, until an assignment or & changes them. */
void declared(int *p) __attribute__((nonnull));
void declared(int *p) { if (p == 0) return; }

void indexed(int *a, int *b) __attribute__((nonnull(2)));
void indexed(int *a, int *b) { if (a == 0 || b != 0) return; if (0 == b) return; }

__attribute__((nonnull(1))) int before(int *p) { return p == (void *)0; }

int old_style(int *p) __attribute__((nonnull(1)));
int old_style(p) int *p; { return !p; }

int own(int *p __attribute__((nonnull))) { return p == 0; }
int inherited(int *p __attribute__((nonnull)));
int inherited(int *p) { return p != 0; }
int both(int *p) __attribute__((nonnull(1)));
int both(int *p __attribute__((nonnull))) { return p == 0; }

int twice(int *p) __attribute__((nonnull(1))) __attribute__((nonnull));
int twice(int *p) { return p == 0; }
int first(int *p) __attribute__((nonnull));
int first(int *p) __attribute__((nonnull(1)));
int first(int *p) { return p == 0; }

__attribute__((nonnull)) int assigned(int *p) { p = 0; return p == 0; }
__attribute__((nonnull)) int addressed(int *p) { int **q = &p; return p == 0 && q; }
__attribute__((nonnull)) int later(int *p) { if (p == 0) return 1; p = 0; return p == 0; }
__attribute__((nonnull)) int stepped(int *p) { p++; p += 1; return p == 0; }
__attribute__((nonnull)) int parenthesized(int *p) { (p) = 0; return p == 0 || (p) == 0; }
__attribute__((nonnull)) int nulls(int *p) { return p == (0) || p == 0L; }

#define NULL_POINTER ((void *)0)
#define IS_NULL(x) ((x) == 0)
#define COMPARE(x) (x == 0)
__attribute__((nonnull)) int macros(int *p) {
  return p == NULL_POINTER || IS_NULL(p) || COMPARE(p);
}

__attribute__((nonnull)) int truth(int *p) { if (p) return 1; return p ? 2 : 3; }
__attribute__((nonnull)) _Bool converted(int *p) { _Bool b = p; return b; }
__attribute__((nonnull)) int operand(int *p, int c) { return c && p; }

int *made(void) __attribute__((returns_nonnull));
int *made_from(int, int) __attribute__((returns_nonnull));
int calls(int x) {
  if (made() == 0)
    return 1;
  if (!made())
    return 2;
  return (made_from(x + 1, 'a')) != 0;
}

int unrelated(int *p) { return p == 0; }
int other(int *p, int *q) __attribute__((nonnull(1)));
int other(int *p, int *q) { return q == 0; }

/* A prototype's parameter is its own: only a nonnull written on it counts,
   and only within a function's body. */
__attribute__((nonnull)) void shadowed(int *p) { void g(int *p, int a[p == 0]); }
void prototype(int *q) { void g(int *p __attribute__((nonnull)), int a[p == 0]); }
void file_scope(int *p __attribute__((nonnull)), int a[p == 0]);

/* A conversion to _Bool looks through parentheses; a comparison only to a call. */
__attribute__((nonnull)) int converted_parentheses(int *p) {
  if ((p))
    return 1;
  if ((p) == 0)
    return 2;
  return !(p);
}
int parenthesized_calls(void) {
  if (((made())))
    return 1;
  if ((made()) == 0)
    return 2;
  return !(made());
}

/* The call's function operand names its callee through *, & and
   parentheses, not through a cast. */
typedef int *(*maker)(void);
int called_through(void) {
  if ((*made)())
    return 1;
  if ((**made)() == 0)
    return 2;
  if ((&made)())
    return 3;
  if (((maker)made)())
    return 4;
  return (*&made)() != 0;
}
