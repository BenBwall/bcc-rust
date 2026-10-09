// errors: 1
typedef int V __attribute__((vector_size(16))); void f(V v,int i) { ((__builtin_shufflevector))(v,v,i,1,2,3); (__builtin_shufflevector)(v,v,0,1,2,3); ((__builtin_elementwise_abs))(v); } int following;
