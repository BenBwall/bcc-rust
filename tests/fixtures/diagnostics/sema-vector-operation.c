typedef int V __attribute__((vector_size(16)));
typedef float F __attribute__((vector_size(16)));
void f(V v, F a, long l) { v=v+l; a=a%a; v=!v; int *p=&v[0]; }
int following;
