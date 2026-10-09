void f(_Atomic int *atomic, int *plain, const _Atomic int *constant,
       _Atomic(float) *floating, float *non_atomic_float) {
    __c11_atomic_load(plain, 0);
    __c11_atomic_store(constant, 1, 0);
    __c11_atomic_compare_exchange_strong(atomic, floating, 1, 5, 0);
    __c11_atomic_fetch_and(floating, 1, 0);
    __atomic_load_n(atomic, 0);
    __atomic_load(atomic, plain, 0);
    __sync_fetch_and_add(non_atomic_float, 1);
    __c11_atomic_load(atomic);
    __atomic_store_n(plain, plain, 0);
}
int following;
