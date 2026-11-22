/* TCC reports a line's warnings as it compiles them. */
int r(int c, int *p, char *q) { return c ? p : q; }
int i(int c, int *p, char *q) { int v = c ? p : q; return v; }
void take(long);
void a(int c, int *p, char *q) { take(c ? p : q); }
