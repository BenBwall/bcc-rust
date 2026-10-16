struct P { int x, y; };
union U { int i; struct P p; };
struct P whole = { .x = 1, .x = 2 };
void f(struct P p) { struct P part[1] = { [0] = p, [0].y = 3 }; (void)part; }
union U member = { .p.x = 1, .i = 2 };
struct F { int n; int v[]; };
static struct F flexible = { .v = { 1, 2, 3 }, .n = 1, .v = { 4 } };
int following_valid;
