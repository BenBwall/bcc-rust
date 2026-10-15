void assign(char *p, float *q) { p = q; }
void initialize(float *q) { char *p = q; }
void accept(char *p);
void argument(float *q) { accept(q); }
char *result(float *q) { return q; }
void function_assign(int (*p)(int), int (*q)(double)) { p = q; }
void integer_pointer(int *p, int q) { p = q; }
void pointer_integer(int p, int *q) { p = q; }
void nested_sign(int **p, unsigned **q) { p = q; }
int following_valid;
