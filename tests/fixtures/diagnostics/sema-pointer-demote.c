void accept(char *p);
void argument(float *q) { accept(q); }
void function_assign(int (*p)(int), int (*q)(double)) { p = q; }
void integer_pointer(int *p, int q) { p = q; }
int following_valid;
