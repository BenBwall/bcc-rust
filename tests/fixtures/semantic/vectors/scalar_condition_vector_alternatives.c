// errors: 0
typedef int V __attribute__((vector_size(16))); typedef int A __attribute__((vector_size(16),aligned(1))); V f(int c,V a,V b,A d) { _Static_assert(__builtin_types_compatible_p(__typeof__(c?a:b),V),"conditional vector type"); _Static_assert(__builtin_types_compatible_p(__typeof__(c?a:d),V),"compatible alternatives"); V v=c?a:b; return c?v:d; } int following;
