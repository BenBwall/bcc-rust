/* GNU Other Builtins: compile-time selections preserve builtin designation. */
static int user(unsigned x) { return (int)x + 6; }
int main(void) {
    if (__builtin_choose_expr(1, __builtin_clz, __builtin_ctz)(1u) != 31)
        return 1;
    if ((__extension__ __builtin_choose_expr(0, __builtin_clz,
            __builtin_choose_expr(1, __builtin_ctz, __builtin_clz)))(8u) != 3)
        return 2;
    if (_Generic(1, int: __builtin_popcount, default: __builtin_clz)(7u) != 3)
        return 3;
    {
        int (*__builtin_clz)(unsigned) = user;
        if (__builtin_choose_expr(1, __builtin_clz, __builtin_clz)(1u) != 7)
            return 4;
        if (_Generic(1, int: (__builtin_clz), default: user)(2u) != 8)
            return 5;
        if (&__builtin_clz == 0)
            return 6;
    }
    return 0;
}
