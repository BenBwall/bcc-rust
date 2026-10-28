int n;
static const int k = 8;
_BitInt(n) runtime;
_BitInt((int){8}) literal;
_BitInt(k) constant_object;
_BitInt(2.0) floating;
_BitInt(1) signed_one;
unsigned _BitInt(1) unsigned_one;
unsigned _BitInt(0) unsigned_zero;
_BitInt(0) zero;
_BitInt(-1) negative;
_BitInt((signed char)-1) negative_char;
_BitInt(65535) gcc_maximum;
_BitInt(65536) above_gcc_maximum;
_BitInt(4294967298LL) wide;
typedef _BitInt(1) Narrow;
int size = sizeof(_BitInt(0));
int selected = _Generic(0, _BitInt(1): 1, default: 0);
void f(void) {
    (void)(_BitInt(0))3;
    int m = 4;
    (void)sizeof(_BitInt(m));
}
_BitInt(3 + 5) sum;
_BitInt((char)300) folded;
enum { W = 9 };
_BitInt(W) enumerator;
int following_valid;
