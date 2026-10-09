void f(_Atomic int *atomic, int *expected, int order, const int *qualified) {
    __c11_atomic_load(atomic, 3);
    __c11_atomic_load(atomic, 4);
    __c11_atomic_store(atomic, 1, 1);
    __c11_atomic_store(atomic, 1, 2);
    __c11_atomic_store(atomic, 1, 4);
    __c11_atomic_compare_exchange_strong(atomic, expected, 1, 0, 3);
    __c11_atomic_compare_exchange_weak(atomic, expected, 1, 0, 4);
    __c11_atomic_exchange(atomic, 1, 99);
    __c11_atomic_load(atomic, order);
    __c11_atomic_compare_exchange_strong(atomic, expected, 1, 0, 5);
    __c11_atomic_thread_fence(99);
    __c11_atomic_compare_exchange_strong(atomic, qualified, 1, 5, 0);
    __atomic_store(expected, qualified, 0);
}
int following;
