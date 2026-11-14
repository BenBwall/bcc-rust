void d1(void) { char c; if (0) c = 1000; (void)c; }
void d2(void) { (void)sizeof((char){1000}); }
void d3(void) { char c; while (0) c = 1001; (void)c; }
void d4(void) { char c; c = 0 && (c = 1002); c = 1 || (c = 1003); }
void d5(void) { char c; return; c = 1004; }
void d6(void) { char c; c = _Generic(0, int: 1, default: (c = 1005)); }
void d7(void) { char c; __typeof__(c = 1006) x; (void)x; }
void d8(void) { char c; c = 0 ? (c = 1007) : 0; }
void d9(int a) { char c; if (a) c = 1008; (void)c; }
void d10(void) { char c; (void)__builtin_choose_expr(1, c = 1009, c = 1010); }
void d11(void) { char c; (void)__builtin_constant_p(c = 1011); }
void d12(void) { char c; (void)_Generic(0, long: 0, int: (c = 1012)); }
char e1(int a) { return a ? 1000 : 0; }
char e2(int a) { return a ? 0 : 1001; }
char e3(int a) { return 0 ? 1002 : a; }
char e4(int a) { return 1 ? a : 1003; }
char e5(int a, int b) { return a ? (b ? 1004 : 0) : 1005; }
char e6(void) { return 1 ? 1006 : 0; }
char e7(void) { return 0 ? 1007 : 0; }
void e8(int a) { char c = a ? 1008 : 0; c = a ? 1009 : 0; (void)c; }
void e9(char);
void e10(int a) { e9(a ? 1010 : 0); }
char e11(int a) { return (a ? 1011 : 0); }
char e12(int a) { return a ? (1012) : 0; }
char e13(int a) { return a ? 'a' : 255; }
unsigned char e14(int a) { return a ? -1 : 256; }
char e15(int a) { return a ? (char)1015 : 0; }
char e16(int a) { return (a, a ? 1016 : 0); }
long e17(int a) { return a ? 2147483648 : 0; }
int e18(int a) { return a ? 2147483648 : 0; }
char e19(int a) { return _Generic(0, int: a ? 1019 : 0); }
char e20(int a) { return __extension__ (a ? 1020 : 0); }
char e21(int a) { return a ?: 1021; }
char e22(void) { return 1 ?: 1022; }
char e23(int a) { return a ? (a ? 1023 : 0) : (0 ? 1024 : 1025); }
char e24(int a) { return a ? 1000 + 26 : 0; }
char g1 = 0 ? 1027 : 0;
char g2 = 1 ? 1028 : 0;
char g3 = 0 ?: 1029;
char g4 = 999 + 31;
unsigned long w1 = (unsigned __int128)1 << 127;
long w2 = (unsigned __int128)-1;
unsigned char w3 = (unsigned __int128)-1;
unsigned long w4 = (unsigned __int128)-1;
__int128 w5 = (unsigned __int128)-1;
unsigned long w6 = ((unsigned __int128)1 << 64) + 5;
long w7 = (__int128)1 << 100;
long w8 = -((__int128)1 << 100);
unsigned long w9 = (unsigned __int128)0xffffffffffffffffULL;
unsigned char w10 = 256u;
unsigned char w11 = 256;
