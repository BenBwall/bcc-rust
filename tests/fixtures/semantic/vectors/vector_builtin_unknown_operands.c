// errors: 5
typedef int V __attribute__((vector_size(16))); void f(V v,int x){ __builtin_shufflevector(v,missing,0); __builtin_shufflevector(missing2,v,0); __builtin_shufflevector(v,v,missing3); __builtin_elementwise_abs(missing4); __builtin_elementwise_min(x,missing5); __builtin_shufflevector(v,v,0,1,2,3); __builtin_elementwise_abs(x); } int following;
