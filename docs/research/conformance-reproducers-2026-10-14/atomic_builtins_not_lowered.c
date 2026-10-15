/* `_Atomic` objects, <stdatomic.h> operations and the GNU __atomic_* and
   __sync_* builtins are typed by semantic analysis but not lowered:
   "not yet supported by lowering: atomic types" and "construct not supported
   by code generation: its type was not analyzed". Clang prints "42 33 9 5 5". */
#include <stdio.h>
#include <stdatomic.h>
int main(void) {
    _Atomic int x = 5;
    atomic_fetch_add(&x, 3);
    int e = 8;
    atomic_compare_exchange_strong(&x, &e, 42);
    _Atomic long long y = 1;
    y += 10;
    y *= 3;
    int v = 1;
    __sync_fetch_and_add(&v, 4);
    int o = __sync_val_compare_and_swap(&v, 5, 9);
    long l = 3;
    __atomic_add_fetch(&l, 2, __ATOMIC_SEQ_CST);
    printf("%d %lld %d %d %ld\n", atomic_load(&x), (long long)y, v, o, l);
    return 0;
}
