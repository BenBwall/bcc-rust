/* GCC's operand checks of the atomic builtins. A generic builtin's pointers
   must point to objects of the first operand's size, whatever their types,
   and one written through may not point to a const object; a value operand
   converts as by a cast; `__atomic_compare_exchange_n`'s expected value is
   an argument of `__atomic_compare_exchange_4`'s `void *` parameter. */
int i;
unsigned u;
long l;
float f;
const int ci;
volatile int vi;
int *ip;
const int *cip;
long *lp;
void *vp;
const void *cvp;
int (*a)[3];
const int (*c)[3];
struct S { int x; } s;
struct T { int y; } t;

void generic(void) {
    __atomic_store(&i, &u, 0);
    __atomic_store(&i, &f, 0);
    __atomic_store(&a, &c, 0);
    __atomic_store(&s, &t, 0);
    __atomic_load(&c, &a, 0);
    __atomic_exchange(&ip, &cip, &cip, 0);
    (void)__atomic_compare_exchange(&i, &u, &f, 0, 0, 0);
}

void sizes(void) {
    __atomic_store(&i, &l, 0);
    __atomic_load(&i, vp, 0);
    __atomic_exchange(&i, &i, &l, 0);
    (void)__atomic_compare_exchange(&i, &i, &l, 0, 0, 0);
    __atomic_store(&i, i, 0);
    __atomic_store(&i, generic, 0);
}

void qualifiers(void) {
    __atomic_load(&i, &ci, 0);
    __atomic_store(&i, &ci, 0);
    __atomic_store(&i, &vi, 0);
    __atomic_store(&ci, &i, 0);
    (void)__atomic_compare_exchange(&i, &ci, &ci, 0, 0, 0);
}

void expected(void) {
    (void)__atomic_compare_exchange_n(&i, &u, 1, 0, 0, 0);
    (void)__atomic_compare_exchange_n(&i, &f, 1, 0, 0, 0);
    (void)__atomic_compare_exchange_n(&ip, &lp, ip, 0, 0, 0);
    (void)__atomic_compare_exchange_n(&i, &ci, 1, 0, 0, 0);
    (void)__atomic_compare_exchange_n(&i, i, 1, 0, 0, 0);
}

void values(void) {
    __atomic_store_n(&a, c, 0);
    __atomic_store_n(&ip, cip, 0);
    __atomic_store_n(&ip, cvp, 0);
    __atomic_store_n(&ip, lp, 0);
    __atomic_store_n(&ip, i, 0);
    (void)__atomic_exchange_n(&ip, cip, 0);
    (void)__sync_val_compare_and_swap(&ip, cip, cip);
    (void)__sync_fetch_and_add(&a, 1);
    (void)__sync_sub_and_fetch(&ip, 1);
    (void)__sync_fetch_and_or(&a, 1);
    (void)__atomic_fetch_or(&ip, 1, 0);
    __atomic_clear(&a, 0);
}

/* A function pointer is an argument of `__atomic_clear`'s or
   `__atomic_test_and_set`'s `volatile void *` parameter. */
void (*fp)(void);

void flags(void) {
    __atomic_clear(values, 0);
    (void)__atomic_test_and_set(fp, 0);
}

int following_valid;
