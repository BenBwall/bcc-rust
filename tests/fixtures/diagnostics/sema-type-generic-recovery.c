void choose(void) { int x = __builtin_choose_expr(missing, 1, 2); int following; }
void jump(int n) { goto L; _Atomic(int (*)[n]) p; L: ; int following; }
void association(int n) {
    typedef int A[n]; typedef _Atomic(A*) P;
    (void)_Generic(0, P: 1, default: 0); int following;
}
typedef int V __attribute__((vector_size(16)));
int *lane(V v) { return &_Generic(1, int: v[0]); }
int *choice_lane(V v) { return &__builtin_choose_expr(1, v[0], 0); }
double runtime_real(void);
int runtime_integer(void);
double imaginary_real = __imag__ runtime_real();
int imaginary_integer = __imag__ runtime_integer();
enum E { imaginary_ice = __imag__ runtime_integer() };
_Complex double z;
double *real_address = &__real__ z;
double *imaginary_address = &__imag__ z;
_Atomic(_Bool) initialized = &z;
int following;
