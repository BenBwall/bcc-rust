#define U(n,p) union U##n { union U##p a, b; };
union U0 { int a, b; };
U(1,0) U(2,1) U(3,2) U(4,3) U(5,4) U(6,5) U(7,6)
U(8,7) U(9,8) U(10,9) U(11,10) U(12,11) U(13,12) U(14,13)
U(15,14) U(16,15) U(17,16) U(18,17) U(19,18) U(20,19) U(21,20)
U(22,21) U(23,22) U(24,23) U(25,24) U(26,25) U(27,26) U(28,27)
U(29,28) U(30,29) U(31,30) U(32,31) U(33,32) U(34,33)
typedef char size_check[sizeof(union U34) == 4 ? 1 : -1];
int f(union U34 x) { return 0; }
int simple(union U0 x) { return x.a; }

#define VALUE(x) ((x).a.a.a.a.a.a.a.a.a.a.a.a.a.a.a.a.a.a.a.a.a.a.a.a.a.a.a.a.a.a.a.a.a.a.a)
int read_nested(union U34 x) { return VALUE(x); }

union U34 global = { 7 };
int main(void) {
    union U34 zero = { 0 }, local = { 11 };
    union U0 value = { 42 };
    return f(local) || VALUE(zero) || VALUE(global) != 7 || VALUE(local) != 11
        || read_nested(local) != 11 || simple(value) != 42;
}
