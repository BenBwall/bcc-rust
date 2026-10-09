// errors: 3
typedef int V __attribute__((vector_size(16))); void f(_Atomic(V) a) {a[0]=1; a+1; 1+a;} int following;
