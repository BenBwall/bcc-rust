// errors: 0
typedef __float128 V __attribute__((vector_size(32))); V f(V v,__float128 x) { _Static_assert(__builtin_types_compatible_p(__typeof__(v+x),V),"binary128 vector type"); _Static_assert(__builtin_types_compatible_p(__typeof__(v+1.0q),V),"literal vector type"); V a=v+x; V b=v+1.0000000000000000000000000000000002q; V c=v+1.0L; return a+b+c; } int following;
