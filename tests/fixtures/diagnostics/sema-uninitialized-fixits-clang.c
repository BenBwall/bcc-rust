/* The edits Clang suggests with its uninitialized-variable notes, as
   -fdiagnostics-parseable-fixits writes them: the zero for each type, the
   removal of an 'if' or '?:' assumed to go one way, the condition a loop
   or '&&' and '||' becomes. */
int g(int);
int then_kept(int a) { int x; if (a) x = 1; else g(0); return x; }
int else_kept(int a) { int x; if (a) g(0); else x = 1; return x; }
int whole(int a) { int x; if (a) return 0; return x; }
double arm(int a) { double x; a ? (x = 1) : 0; return x; }
char and_kept(int a, int b) { char x; if (a && b) x = 1; return x; }
long or_value(int a, int b) { long x; if (a || b) return 0; return x; }
int while_entered(int n) { int x; while (n) { x = 1; n--; } return x; }
int for_exited(int n) { int x; for (; n;) { x = 1; break; } return x; }
int do_exited(int n) { int x; do { if (g(n)) { x = 1; break; } } while (n--); return x; }
