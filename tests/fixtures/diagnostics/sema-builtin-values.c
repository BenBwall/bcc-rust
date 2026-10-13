void f(void *p, int n, double probability) {
    __builtin_object_size(p, n);
    __builtin_object_size(p, 4);
    __builtin_dynamic_object_size(p, -1);
    __builtin_expect_with_probability(1, 1, probability);
    __builtin_expect_with_probability(1, 1, 1.5);
    __builtin_assume_aligned(p, n);
    __builtin_assume_aligned(p, 3);
    __builtin_prefetch(p, n, 3);
    __builtin_prefetch(p, 2, 3);
    __builtin_prefetch(p, 0, 4);
    __builtin_object_size(p, p);
    __builtin_expect_with_probability(1, 1, p);
    __builtin_assume_aligned(p, p);
}
int following;
