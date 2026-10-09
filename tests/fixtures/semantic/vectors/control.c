// errors: 0
typedef int V __attribute__((vector_size(16))); V f(V a,V b){(void)a; return a+b;} int following;
