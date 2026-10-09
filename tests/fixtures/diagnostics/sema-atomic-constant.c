_Static_assert(sizeof(_Atomic(int)) == 5, "atomic size is checked");
_Static_assert((_Atomic(int))1 + 2 == 3, "atomic casts are not ICEs");
_Static_assert(_Generic(1, int:(_Atomic(int))1, default:0), "selected atomic cast is not an ICE");
int runtime;
_Static_assert(__atomic_always_lock_free(runtime, 0), "runtime size is not an ICE");
_Static_assert(__c11_atomic_is_lock_free(16), "non-lock-free query is runtime");
struct Bad { _Atomic int field : 1; };
void f(_Atomic(void *) *object) { __c11_atomic_fetch_add(object, 1, 0); }
int following;
