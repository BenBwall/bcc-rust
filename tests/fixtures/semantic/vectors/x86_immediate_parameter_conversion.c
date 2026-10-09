// errors: 0
typedef int V __attribute__((vector_size(16))); V f(V v) { V a=__builtin_ia32_pshufd(v,4294967296ULL); V b=__builtin_ia32_pshufd(v,4294967551ULL); V c=__builtin_ia32_pshufd(v,0); return a+b+c;} V g(V v,const int *p){return __builtin_ia32_gatherd_d(v,p,v,v,260);} int following;
