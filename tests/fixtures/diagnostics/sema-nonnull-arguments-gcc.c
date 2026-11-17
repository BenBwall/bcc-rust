/* Null arguments for nonnull parameters (-Wnonnull). */
void use(int *p, int *q) __attribute__((nonnull(2)));
void use(int *p, int *q);
void all(int *a, int *b) __attribute__((nonnull));
void defined(int *p) __attribute__((nonnull));
void defined(int *p) { (void)p; }
void counted(int n, int *p) __attribute__((nonnull));
void list(int *p, ...) __attribute__((nonnull));
void late(int *p);
void late(int *p) __attribute__((nonnull));

void calls(int *p) {
  use(0, 0);
  use(0, (int *)0);
  use(p, p);
  (use)(0, 0);
  all(0, 0);
  defined(0);
  use(0, (void *)(1 - 1));
  counted(0, 0);
  list(0, 0);
  late(0);
}
