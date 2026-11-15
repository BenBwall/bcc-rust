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
char gc;
int u1 = _Generic(0, int: 0, default: (gc = 1100));
int u2 = _Generic(0, int: 0, long: (gc = 1101));
int u3 = _Generic(0, int: _Generic(0, int: 0, default: (gc = 1102)), default: 0);
char u4 = _Generic(0, int: 0, default: 1103);
int u5 = __builtin_choose_expr(1, 0, gc = 1104);
char u6 = _Generic(0, int: 1105, default: 0);
void *u7 = _Generic(0, int: &(char){1106}, default: 0);
void *u8 = _Generic(0, long: &(char){1107}, default: (void *)0);
int u9 = _Generic(0, long: _Generic(0, int: (gc = 1108), default: 0), default: 0);
int u10 = __builtin_choose_expr(0, _Generic(0, int: (gc = 1109), default: 0), 0);
void *u11 = __builtin_choose_expr(1, _Generic(0, int: &(char){1110}, default: (gc = 1111)), 0);
void *u12 = __builtin_choose_expr(0, 0, &(char){1112});
int u13 = _Generic(0, int: __builtin_choose_expr(1, 0, gc = 1113), default: 0);
int u14 = _Generic(0, long: __builtin_choose_expr(1, &(char){1114}, 0), default: 0);
int u15[] = { _Generic(0, int: 0, default: (gc = 1115)), __builtin_choose_expr(0, gc = 1116, 1) };
char u16 = __builtin_choose_expr(1, 1117, 0);
char u17 = __builtin_choose_expr(1, 0, 1118);
void *u18 = _Generic(0, int: _Generic(0, int: &(char){1119}, default: (gc = 1120)), default: (gc = 1121));
void b1(void) { static int s = _Generic(0, int: 0, default: (gc = 1200)); (void)s; }
void b2(void) { static int s = __builtin_choose_expr(1, 0, gc = 1201); (void)s; }
void b3(void) { static int s = _Generic(0, int: _Generic(0, int: 0, default: (gc = 1202)), default: 0); (void)s; }
void b4(void) { static char s = _Generic(0, long: 1203, default: 0); (void)s; }
void b5(void) { static int s = _Generic(0, long: 0, default: _Generic(0, int: 0, default: (gc = 1204))); (void)s; }
void b6(void) { static char s = _Generic(0, int: 1205, default: 1206); (void)s; }
void b7(void) { static int s = __builtin_choose_expr(0, _Generic(0, int: (gc = 1207), default: 0), 0); (void)s; }
