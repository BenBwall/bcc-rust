/* The functions of TCC's runtime library, libtcc1.a, that bcc's own headers
   declare as functions: the fences and flag operations of <stdatomic.h>.
   Their macros expand a call to a builtin, so a program reaches one of these
   only through its address or a parenthesized name. TCC links libtcc1.a
   into every program; bcc compiles this source for the target and gives a
   unit under --tcc-mode the bodies of the ones it references
   (tcc_runtime.rs). Each signature is the IR signature of the declaration in
   src/headers/stdatomic.h: an enum memory_order is an int, and a
   volatile atomic_flag * a pointer to its one _Bool. Like libtcc1's, each
   function ignores its order and orders sequentially consistently, the
   strongest order, which every weaker one allows (C11 7.17.3). */

void atomic_thread_fence(int order) {
    (void)order;
    __atomic_thread_fence(__ATOMIC_SEQ_CST);
}

void atomic_signal_fence(int order) {
    (void)order;
    __atomic_signal_fence(__ATOMIC_SEQ_CST);
}

_Bool atomic_flag_test_and_set(volatile _Bool *object) {
    return __atomic_test_and_set(object, __ATOMIC_SEQ_CST);
}

_Bool atomic_flag_test_and_set_explicit(volatile _Bool *object, int order) {
    (void)order;
    return __atomic_test_and_set(object, __ATOMIC_SEQ_CST);
}

void atomic_flag_clear(volatile _Bool *object) {
    __atomic_clear(object, __ATOMIC_SEQ_CST);
}

void atomic_flag_clear_explicit(volatile _Bool *object, int order) {
    (void)order;
    __atomic_clear(object, __ATOMIC_SEQ_CST);
}
