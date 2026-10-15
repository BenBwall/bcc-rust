int equal(int *p, unsigned *q) { return p == q; }
int order(int *p, float *q) { return p < q; }
void sign(int *p, unsigned *q) { p = q; }
void discard(int *p, const int *q) { p = q; }
void nested(const int **p, int **q) { p = q; }
int integer_equal(int *p, int q) { return p == q; }
int integer_order(int *p, int q) { return p < q; }
int function_order(int (*p)(int), int (*q)(int)) { return p < q; }
int distinct_function_order(int (*p)(int), int (*q)(double)) { return p < q; }
int following_valid;
