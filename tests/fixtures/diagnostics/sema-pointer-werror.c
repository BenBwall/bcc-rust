int equal(int *p, unsigned *q) { return p == q; }
void sign(int *p, unsigned *q) { p = q; }
void nested(const int **p, int **q) { p = q; }
int following_valid;
