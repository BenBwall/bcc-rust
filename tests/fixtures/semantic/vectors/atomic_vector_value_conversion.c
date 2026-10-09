// errors: 0
typedef int V __attribute__((vector_size(16))); V f(_Atomic(V) a,V b,int x,_Atomic(int) i) { a=b; a+=b; _Static_assert(__builtin_types_compatible_p(__typeof__(a+=b),V),"assignment value type"); _Static_assert(__builtin_types_compatible_p(__typeof__((0,a)),V),"comma value type"); V loaded=b+i; int indexed=b[i]; (void)indexed; V c=a+b; V d=b+a; return c+d+(b+x); } int following;
