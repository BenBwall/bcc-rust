enum e { A, B, C };
enum f { X = 1, Y = 2, Z = 4 };
enum g { G1 = 1, G2 = 1 };
int a(enum e x) { switch (x) { case A: return 1; } return 0; }
int b(enum e x) { switch (x) { case A: case B: return 1; } return 0; }
int c(enum e x) { switch (x) { case A: return 1; default: return 0; } }
int d(enum e x) { switch (x) { case A: case B: case C: return 1; } return 0; }
int e(enum e x) { switch (x) { case A: case B: case C: case 5: return 1; } return 0; }
int f(enum e x) { switch (x) { case A: case 7: return 1; default: return 0; } }
int g(int x) { switch (x) { case A: return 1; } return 0; }
int h(enum f x) { switch (x) { case X: case Y: return 1; } return 0; }
int i(enum f x) { switch (x) { case X: case Y: case Z: case X | Y: return 1; } return 0; }
int k(enum e x) { switch ((int)x) { case A: return 1; } return 0; }
int l(enum e x) { switch (x + 0) { case A: return 1; } return 0; }
int m(enum g x) { switch (x) { case G1: return 1; } return 0; }
int n(enum e x) { switch (x) { case A ... B: return 1; } return 0; }
int o(enum e x) { switch (x) { } return 0; }
int p(enum e x) { switch (x) { case A: case B: break; case C: break; case C+1: break; } return 0; }
int q(unsigned char x) { switch (x) { case 300: return 1; case -1: return 2; } return 0; }
int r(_Bool x) { switch (x) { case 2: return 1; } return 0; }
int s(enum e x) { switch (x) { case A: case B: case C: return 1; default: return 0; } }
