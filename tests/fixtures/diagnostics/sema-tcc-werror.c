/* TCC's -Werror makes its first warning the error it stops at. */
int f(void);
static int f(void) { return 0; }
int x;
static int x;
