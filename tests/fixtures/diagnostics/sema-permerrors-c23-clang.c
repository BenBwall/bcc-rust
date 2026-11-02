/* Clang refuses implicit int and implicit declarations outright in C23. */
static x = 1;
int f(void) { return undeclared(x); }
