/* GCC Other Builtins; C99 7.15.1: no va_arg independently forces storage. */
static int selected(int n, ...) {
    __builtin_va_list ap, copy;
    __builtin_choose_expr(1, __builtin_va_start, __builtin_va_start)(ap, n);
    __builtin_choose_expr(0, __builtin_va_copy, __builtin_va_copy)(copy, ap);
    __builtin_choose_expr(1, __builtin_va_end, __builtin_va_end)(copy);
    __builtin_choose_expr(0, __builtin_va_end, __builtin_va_end)(ap);
    return n;
}
static int extended(int n, ...) {
    __builtin_va_list ap, copy;
    (__extension__ __builtin_va_start)(ap, n);
    (__extension__ __builtin_choose_expr(1, __builtin_va_copy, __builtin_va_copy))(copy, ap);
    (__extension__ __builtin_va_end)(copy);
    (__extension__ __builtin_choose_expr(0, __builtin_va_end, __builtin_va_end))(ap);
    return n;
}
int main(void) { return selected(3, 7) != 3 || extended(5, 9) != 5; }
