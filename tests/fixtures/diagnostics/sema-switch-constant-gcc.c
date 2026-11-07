enum e { A, B, C };
enum u { U = 0xffffffffU, V = 0 };
enum s { S = -1, T = 0 };
enum m { M1, M2 __attribute__((unused)), M3 };
const enum e global = B;
int a(void) { switch ((enum e)A) { case A: return 1; } return 0; }
int b(void) { switch ((enum e)B) { case A: return 1; } return 0; }
int c(void) { switch ((enum e)7) { case A: case B: return 1; } return 0; }
int d(void) { switch (global) { case A: return 1; } return 0; }
int e(void) { const enum e local = C; switch (local) { case A: return 1; } return 0; }
int f(int x) { switch ((x++, (enum e)A)) { case A: return 1; } return 0; }
int g(void) { switch (2 + 1) { case 0: return 1; } return 0; }
int h(enum u x) { switch (x) { case -1: case 0: return 1; } return 0; }
int i(enum u x) { switch (x) { case -2: case -1: case 0: return 1; } return 0; }
int j(enum s x) { switch (x) { case 0xffffffffU: case 0: return 1; } return 0; }
int l(int flag, enum e x, enum e y) { switch (flag ? x : y) { case A: return 1; } return 0; }
int m(int flag, enum e x) { switch (flag ? x : B) { case A: return 1; } return 0; }
int n(int flag, enum e x) { switch (flag ? x : 1) { case A: return 1; } return 0; }
int o(int flag, enum e x, enum e y) { switch (({ flag ? x : y; })) { case A: return 1; } return 0; }
int p(enum m x) { switch (x) { case M1: return 1; } return 0; }
int q(enum e x) { switch (x) { case A: return 1; default: return 0; } }
