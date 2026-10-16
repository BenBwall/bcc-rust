/* GCC Integer Overflow Builtins: the third operand executes only its effects:
   selected arms, evaluated VLA sizeof operands, short circuits and blocks. */
static int f(int *c) { return ++*c; }
int main(void) {
    int c = 0, n = 2;
    int b[n][n];
    if (__builtin_add_overflow_p(1, 2, _Generic(0, int: c++, default: f(&c))) || c != 1) return 1;
    if (__builtin_add_overflow_p(1, 2, __builtin_choose_expr(0, f(&c), c++)) || c != 2) return 2;
    if (__builtin_add_overflow_p(1, 2, (int)sizeof(c++)) || c != 2) return 3;
    if (__builtin_add_overflow_p(1, 2, (int)sizeof(b[c++])) || c != 3) return 4;
    if (__builtin_add_overflow_p(1, 2, (int)sizeof(int[c++])) || c != 4) return 5;
    if (__builtin_add_overflow_p(1, 2, c ?: f(&c)) || c != 4) return 6;
    if (__builtin_add_overflow_p(1, 2, (c = 0) ?: f(&c)) || c != 1) return 7;
    if (__builtin_add_overflow_p(1, 2, c || f(&c)) || c != 1) return 8;
    if (__builtin_add_overflow_p(1, 2, (c - 1) || f(&c)) || c != 2) return 9;
    if (__builtin_add_overflow_p(1, 2, c && (c - 2 ? 0 : f(&c))) || c != 3) return 10;
    if (__builtin_add_overflow_p(1, 2, ({ for (int i = 0; i < 3; i++) c++; c; })) || c != 6) return 11;
    if (__builtin_add_overflow_p(1, 2, (c++, b[0][0] + c)) || c != 7) return 12;
    return 0;
}
