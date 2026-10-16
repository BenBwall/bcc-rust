/* GCC Integer Overflow Builtins: ignore values, retain effects and targets. */
int main(void) {
    int n = 0;
    if (__builtin_add_overflow_p(1, 2, ({ n++; *(int *)0; })) || n != 1) return 1;
    if (__builtin_add_overflow_p(1, 2, ({ n++; tail: *(int *)0; })) || n != 2) return 2;
    if (__builtin_sub_overflow_p(2, 1, ({ goto out; n += 100; out: n++; next: *(int *)0; })) || n != 3) return 3;
    if (__builtin_mul_overflow_p(1, 2, ({ switch (2) {
        case 1: n += 100; break;
        case 2: n++;
        case 3: *(int *)0; break;
        default: n += 100;
    } done: *(int *)0; })) || n != 4) return 4;
    volatile int v = 7;
    if (__builtin_add_overflow_p(1, 2, ({ n++; volatile_tail: v; })) || n != 5) return 5;
    if (__builtin_add_overflow_p(1, 2, ({ switch (9) {
        case 1: n += 100; break;
        default: *(int *)0;
    } n++; *(int *)0; })) || n != 6) return 6;
    return 0;
}
