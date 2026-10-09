#define _CLANG_DISABLE_CRT_DEPRECATION_WARNINGS
#include <stdatomic.h>
#define TYPE(expression, type) _Static_assert(_Generic((expression), type:1, default:0), #expression)
struct Three { char bytes[3]; };
struct Nine { char bytes[9]; };
struct Big { char bytes[17]; };
_Static_assert(sizeof(_Atomic(struct Three)) == 4, "small aggregate size");
_Static_assert(_Alignof(_Atomic(struct Three)) == 4, "small aggregate alignment");
_Static_assert(sizeof(_Atomic(struct Nine)) == 16, "aggregate size");
_Static_assert(_Alignof(_Atomic(struct Nine)) == 16, "aggregate alignment");
_Static_assert(sizeof(_Atomic(struct Big)) == 17, "large aggregate size");
_Static_assert(_Alignof(_Atomic(struct Big)) == 1, "large aggregate alignment");
_Static_assert(sizeof(_Atomic(__int128)) == 16, "int128 size");
_Static_assert(_Alignof(_Atomic(__int128)) == 16, "int128 alignment");
_Static_assert(sizeof(atomic_long) == sizeof(long), "target long");
_Static_assert(_Alignof(atomic_llong) == 8, "long long alignment");
_Static_assert(sizeof(atomic_flag) == 1, "flag size");
_Static_assert(ATOMIC_INT_LOCK_FREE == 2, "int lock-free");
_Static_assert(__atomic_always_lock_free(8, 0), "eight bytes lock-free");
_Static_assert(!__atomic_always_lock_free(16, 0), "sixteen bytes not always lock-free");
_Static_assert(__c11_atomic_is_lock_free(4), "four bytes lock-free");
_Static_assert(__c11_atomic_is_lock_free(0), "zero bytes lock-free");
_Static_assert(__atomic_always_lock_free(0, 0), "zero bytes always lock-free");
_Static_assert(__atomic_always_lock_free(8, (char *)0), "typed null alignment");
_Static_assert(!__atomic_always_lock_free(8, (char *)1), "insufficient alignment");
_Static_assert(__atomic_always_lock_free(8, (long long *)1), "type alignment");
_Static_assert(__atomic_is_lock_free(4, 0), "four bytes lock-free");
atomic_int integer = 1;
atomic_flag flag = ATOMIC_FLAG_INIT;
_Atomic(int *) pointer;
atomic_bool boolean;
atomic_int_least8_t least8; atomic_uint_least8_t uleast8;
atomic_int_least16_t least16; atomic_uint_least16_t uleast16;
atomic_int_least32_t least32; atomic_uint_least32_t uleast32;
atomic_int_least64_t least64; atomic_uint_least64_t uleast64;
atomic_int_fast8_t fast8; atomic_uint_fast8_t ufast8;
atomic_int_fast16_t fast16; atomic_uint_fast16_t ufast16;
atomic_int_fast32_t fast32; atomic_uint_fast32_t ufast32;
atomic_int_fast64_t fast64; atomic_uint_fast64_t ufast64;
atomic_intptr_t intptr; atomic_uintptr_t uintptr;
atomic_size_t size; atomic_ptrdiff_t difference;
atomic_intmax_t maximum; atomic_uintmax_t umaximum;
atomic_char16_t char16; atomic_char32_t char32; atomic_wchar_t wide;
#if __STDC_VERSION__ >= 202311L
#ifdef ATOMIC_VAR_INIT
#error ATOMIC_VAR_INIT must be removed in C23
#endif
atomic_char8_t char8;
_Static_assert(ATOMIC_CHAR8_T_LOCK_FREE == 2, "char8 lock-free");
#else
atomic_int initialized = ATOMIC_VAR_INIT(2);
#endif
void standard_operations(volatile atomic_int *object, int *expected, int order) {
    TYPE(atomic_load(object), int);
    TYPE(atomic_load_explicit(object, order), int);
    atomic_store(object, 1);
    atomic_store_explicit(object, 1, order);
    TYPE(atomic_exchange(object, 1), int);
    TYPE(atomic_exchange_explicit(object, 1, order), int);
    TYPE(atomic_compare_exchange_strong(object, expected, 1), _Bool);
    TYPE(atomic_compare_exchange_weak(object, expected, 1), _Bool);
    TYPE(atomic_compare_exchange_strong_explicit(object, expected, 1, 5, 2), _Bool);
    TYPE(atomic_compare_exchange_weak_explicit(object, expected, 1, 5, 0), _Bool);
    TYPE(atomic_fetch_add(object, 1), int);
    TYPE(atomic_fetch_sub(object, 1), int);
    TYPE(atomic_fetch_and(object, 1), int);
    TYPE(atomic_fetch_or(object, 1), int);
    TYPE(atomic_fetch_xor(object, 1), int);
    TYPE(atomic_fetch_add_explicit(object, 1, 0), int);
    TYPE(atomic_fetch_sub_explicit(object, 1, 0), int);
    TYPE(atomic_fetch_and_explicit(object, 1, 0), int);
    TYPE(atomic_fetch_or_explicit(object, 1, 0), int);
    TYPE(atomic_fetch_xor_explicit(object, 1, 0), int);
    TYPE(atomic_fetch_add(&pointer, 1), int *);
    TYPE(atomic_is_lock_free(object), _Bool);
    TYPE(atomic_flag_test_and_set(&flag), _Bool);
    TYPE(atomic_flag_test_and_set_explicit(&flag, 2), _Bool);
    atomic_flag_clear(&flag);
    atomic_flag_clear_explicit(&flag, 3);
    atomic_thread_fence(order);
    atomic_signal_fence(order);
    atomic_init(object, 1);
    TYPE(kill_dependency(1L), long);
    integer += 2; integer++; --integer;
}

void compiler_operations(int *p, int **pp, float *fp, double *dp,
                         _Atomic(float) *af, _Atomic(double) *ad, int order) {
    int expected=0, desired=1, result;
    struct Three object, input, output;
    TYPE(__atomic_load_n(p, order), int);
    TYPE(__atomic_exchange_n(p, 1, order), int);
    TYPE(__atomic_compare_exchange_n(p, &expected, 1, 0, 5, 0), _Bool);
    __atomic_store_n(p, 1, order);
    __atomic_load(&object, &output, order);
    __atomic_store(&object, &input, order);
    __atomic_exchange(&object, &input, &output, order);
    TYPE(__atomic_compare_exchange(&object, &output, &input, 0, 5, 0), _Bool);
    TYPE(__atomic_test_and_set(p, order), _Bool);
    __atomic_clear((char *)p, order);
    __atomic_clear(&boolean, order);
    __atomic_thread_fence(order); __atomic_signal_fence(order);
    TYPE(__atomic_load_n(fp, order), float);
    TYPE(__atomic_exchange_n(dp, 1.0, order), double);
    TYPE(__atomic_fetch_add(pp, 1, order), int *);
    TYPE(__c11_atomic_fetch_add(af, 1.0f, order), float);
    TYPE(__c11_atomic_fetch_sub(ad, 1.0, order), double);
    TYPE(__c11_atomic_fetch_add(&integer, 1, order), int);
    TYPE(__atomic_fetch_add(p, 1, order), int);
    TYPE(__atomic_add_fetch(p, 1, order), int);
    TYPE(__c11_atomic_fetch_sub(&integer, 1, order), int);
    TYPE(__atomic_fetch_sub(p, 1, order), int);
    TYPE(__atomic_sub_fetch(p, 1, order), int);
    TYPE(__c11_atomic_fetch_and(&integer, 1, order), int);
    TYPE(__atomic_fetch_and(p, 1, order), int);
    TYPE(__atomic_and_fetch(p, 1, order), int);
    TYPE(__c11_atomic_fetch_or(&integer, 1, order), int);
    TYPE(__atomic_fetch_or(p, 1, order), int);
    TYPE(__atomic_or_fetch(p, 1, order), int);
    TYPE(__c11_atomic_fetch_xor(&integer, 1, order), int);
    TYPE(__atomic_fetch_xor(p, 1, order), int);
    TYPE(__atomic_xor_fetch(p, 1, order), int);
    TYPE(__c11_atomic_fetch_nand(&integer, 1, order), int);
    TYPE(__atomic_fetch_nand(p, 1, order), int);
    TYPE(__atomic_nand_fetch(p, 1, order), int);
    TYPE(__c11_atomic_fetch_min(&integer, 1, order), int);
    TYPE(__atomic_fetch_min(p, 1, order), int);
    TYPE(__atomic_min_fetch(p, 1, order), int);
    TYPE(__c11_atomic_fetch_max(&integer, 1, order), int);
    TYPE(__atomic_fetch_max(p, 1, order), int);
    TYPE(__atomic_max_fetch(p, 1, order), int);
    TYPE(__sync_fetch_and_add(p, 1), int);
    TYPE(__sync_add_and_fetch(p, 1), int);
    TYPE(__sync_fetch_and_sub(p, 1), int);
    TYPE(__sync_sub_and_fetch(p, 1), int);
    TYPE(__sync_fetch_and_and(p, 1), int);
    TYPE(__sync_and_and_fetch(p, 1), int);
    TYPE(__sync_fetch_and_or(p, 1), int);
    TYPE(__sync_or_and_fetch(p, 1), int);
    TYPE(__sync_fetch_and_xor(p, 1), int);
    TYPE(__sync_xor_and_fetch(p, 1), int);
    TYPE(__sync_fetch_and_nand(p, 1), int);
    TYPE(__sync_nand_and_fetch(p, 1), int);

    TYPE(__sync_bool_compare_and_swap(p, 0, 1), _Bool);
    TYPE(__sync_val_compare_and_swap(p, 0, 1), int);
    TYPE(__sync_lock_test_and_set(p, 1), int);
    __sync_lock_release(p); __sync_synchronize();
    result=__sync_fetch_and_add(p,1,2,3);
    (void)result; (void)desired;
}
