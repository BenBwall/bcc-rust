/* A function designator initializing a `void *` member of a static object: rejected by bcc with
   "initializer cannot initialize this object"; Clang and GCC accept it (SQLite 3.54 relies on it). */
struct entry { int n; void *p; };
static double twice(double v) { return 2 * v; }
static const struct entry table[] = { {1, twice}, {2, 0} };
int main(void) { return table[0].p != 0 ? 0 : 1; }
