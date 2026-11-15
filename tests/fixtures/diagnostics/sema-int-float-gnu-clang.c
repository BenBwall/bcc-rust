float f1(int a) { return a ?: 16777217; }
float f2(int a) { return __extension__ (a ? 16777217 : 1.0); }
float f3(int a) { return a ? __extension__ 16777219 : 0; }
float f4(void) { return 16777221 ?: 0; }
float f5(void) { return 0 ?: 16777223; }
float f6(void) { return 1 ?: 16777225; }
float f7(int a) { return _Generic(0, int: a ? 16777227 : 0); }
float f8(int a) { return __builtin_choose_expr(1, a ? 16777229 : 0, 0); }
float f9(int a, int b) { return a ?: (b ? 16777231 : 0); }
float f10(int a) { return (a ?: 16777233); }
float f11(int a) { return __extension__ (a ?: 16777235); }
double f12(int a) { return a ?: 9007199254740993; }
float f13(int a) { return a + (a ?: 16777237); }
float f14(float x, int a) { return x + (a ?: 16777239); }
float f15(float x, int a) { return x + __extension__ 16777241; }
float g1 = 0 ?: 16777243;
float g2 = __extension__ 16777245;
