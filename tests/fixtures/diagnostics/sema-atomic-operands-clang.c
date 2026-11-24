/* Clang's operand checks of the atomic builtins: a pointer operand converts
   as a prototype's argument to a pointer to the object's unqualified type,
   whose discarded qualifiers `-pedantic-errors` makes an error, and a null
   one is diagnosed, since the builtin reads or writes through it. */
int i;
const int ci;
volatile int vi;
const void *cvp;
int *ip;
const int *cip;
int (*a)[3];
_Atomic int ai;

void qualifiers(void) {
    __atomic_load(&i, &ci, 0);
    __atomic_store(&i, cvp, 0);
    (void)__atomic_compare_exchange(&i, &ci, &ci, 0, 0, 0);
    (void)__atomic_compare_exchange_n(&i, &vi, 1, 0, 0, 0);
    __atomic_store_n(&ip, cip, 0);
}

void nulls(void) {
    (void)__atomic_compare_exchange_n(&i, 0, 1, 0, 0, 0);
    __atomic_load(&i, 0, 0);
    __atomic_store(&i, (int *)0, 0);
    (void)__c11_atomic_compare_exchange_strong(&ai, 0, 1, 5, 5);
}

void clear(void) {
    (void)__atomic_test_and_set(&a, 0);
    __atomic_clear(&a, 0);
    __atomic_clear(&i, 0);
}

int following_valid;
