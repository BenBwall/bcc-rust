typedef int V __attribute__((vector_size(16)));
void f(V v, int i) { v=__builtin_shufflevector(v,v,i,1,2,3); v=__builtin_convertvector(v,int); }
int following;
