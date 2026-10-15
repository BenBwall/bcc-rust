void convert(int (*p)(int), void *q) { p = q; }
int compare(int (*p)(int), void *q) { return p == q; }
int zero_order(int *p) { return p < 0; }
int following_valid;
