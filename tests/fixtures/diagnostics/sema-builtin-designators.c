void f(void) {
    &__builtin_clz;
    __builtin_ctz;
    &(__builtin_choose_expr(1, __builtin_popcount, __builtin_clz));
    _Generic(1, int: __builtin_clz, default: __builtin_ctz);
}
int following;
