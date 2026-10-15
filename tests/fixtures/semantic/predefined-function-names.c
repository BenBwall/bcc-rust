int printf(const char *, ...);
typedef __SIZE_TYPE__ size_t;
typedef __PTRDIFF_TYPE__ ptrdiff_t;

typedef unsigned long count_t;
typedef int (*callback_t)(int);
typedef struct { int a; } box_t;
typedef const int fixed_t;
struct node { int value; };
union number { int i; float f; };
enum color { RED };
typedef int row_t[3];
typedef void callback_function_t(int);

const char *top_name = __func__;
const char *top_function = __FUNCTION__;
const char *top_pretty = __PRETTY_FUNCTION__;

#define SHOW(label) \
  printf("%s %s|%s|%s %d %d %d\n", label, __func__, __FUNCTION__, \
         __PRETTY_FUNCTION__, (int)sizeof __func__, (int)sizeof __FUNCTION__, \
         (int)sizeof __PRETTY_FUNCTION__)

void unprototyped() { SHOW("unprototyped"); }
void no_parameters(void) { SHOW("no_parameters"); }
int scalars(char c, signed char sc, unsigned char uc, short s, unsigned short us, int i,
            unsigned u, long l, unsigned long ul, long long ll, unsigned long long ull) {
  SHOW("scalars");
  return 0;
}
void reals(float f, double d, _Bool b) { SHOW("reals"); }
void pointers(char *p, const char *cp, char *const pc, const char *const cpc, char **pp,
              volatile int *vp, int *restrict rp, void *v) {
  SHOW("pointers");
}
void tags(struct node n, struct node *np, union number u, enum color c, const struct node *cn) {
  SHOW("tags");
}
void typedefs(count_t c, size_t z, callback_t cb, callback_t *cbp, box_t b, box_t *bp,
              fixed_t f, const callback_t cc, ptrdiff_t pd) {
  SHOW("typedefs");
}
void arrays(int a[3], int b[], int m[2][3], int n, int vla[n]) { SHOW("arrays"); }
void functions(int (*f)(int, char), void (*g)(void), int (*h)(), int (*k)(int, ...)) {
  SHOW("functions");
}
void derived(const count_t *a, count_t **b, volatile count_t c, row_t r, row_t *rp,
             callback_function_t *cf, _Atomic(int) *at, __int128 w) {
  SHOW("derived");
}
void variadic(int n, ...) { SHOW("variadic"); }
void only_variadic_after_pointer(const char *format, ...) { SHOW("only_variadic_after_pointer"); }
static const char *pointer_result(int x) {
  SHOW("pointer_result");
  return 0;
}
count_t typedef_result(void) {
  SHOW("typedef_result");
  return 0;
}
struct node *tag_result(const struct node *n) {
  SHOW("tag_result");
  return 0;
}
int (*function_result(int a))(char) {
  SHOW("function_result");
  return 0;
}
int (*array_pointer_result(void))[4] {
  SHOW("array_pointer_result");
  return 0;
}
static inline unsigned inline_function(unsigned a) {
  SHOW("inline_function");
  return a;
}
int old_style(a, b) int a; char b; { SHOW("old_style"); return a + b; }
int main_like_name(int argc, char **argv) {
  SHOW("main_like_name");
  return 0;
}

void blocks(int x) {
  {
    int __attribute__((unused)) y = 0;
    SHOW("block");
    {
      SHOW("nested block");
    }
  }
  printf("expression %s\n", ({ __PRETTY_FUNCTION__; }));
  for (int i = 0; i < 1; i++) printf("for %s\n", __FUNCTION__);
  /* One object serves __func__ and __FUNCTION__, and every use of each. */
  printf("same %d %d %d\n", __func__ == __FUNCTION__, __func__ == __func__,
         __PRETTY_FUNCTION__ == __PRETTY_FUNCTION__);
  printf("elements %c %c %c\n", __func__[0], __FUNCTION__[1], __PRETTY_FUNCTION__[2]);
  printf("pointer %d\n", *(&__PRETTY_FUNCTION__[0]) == 'v');
}

int main(void) {
  unprototyped();
  no_parameters();
  scalars(0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0);
  reals(0, 0, 0);
  pointers(0, 0, 0, 0, 0, 0, 0, 0);
  struct node n = {0};
  union number u = {0};
  tags(n, 0, u, RED, 0);
  box_t b = {0};
  typedefs(0, 0, 0, 0, b, 0, 0, 0, 0);
  arrays(0, 0, 0, 1, 0);
  functions(0, 0, 0, 0);
  derived(0, 0, 0, 0, 0, 0, 0, 0);
  variadic(1);
  only_variadic_after_pointer("");
  pointer_result(0);
  typedef_result();
  tag_result(0);
  function_result(0);
  array_pointer_result();
  inline_function(0);
  old_style(0, 0);
  main_like_name(0, 0);
  blocks(0);
  printf("file [%s] [%s] [%s] %d\n", top_name, top_function, top_pretty,
         (int)sizeof __PRETTY_FUNCTION__);
  SHOW("main");
  return 0;
}
