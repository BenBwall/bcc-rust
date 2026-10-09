// errors: 0
typedef int V __attribute__((vector_size(16)));
typedef V *P __attribute__((aligned(1)));
typedef V VA[2] __attribute__((aligned(1)));
typedef int *Q __attribute__((aligned(1)));
typedef V *Plain;
_Static_assert(_Alignof(V) == 16, "vector alignment");
_Static_assert(_Alignof(*(Plain)0) == 16, "pointee alignment");
typedef V W __attribute__((aligned(64)));
_Static_assert(sizeof(W) == 16 && _Alignof(W) == 64, "vector-only alignment");
typedef float U __attribute__((__vector_size__(16), __aligned__(1)));
_Static_assert(sizeof(U) == 16 && _Alignof(U) == 1, "unaligned intrinsic vector");
int following;
