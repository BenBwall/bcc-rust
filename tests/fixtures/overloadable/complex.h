/* C99 complex library contracts, independent of the host ABI. */
#define COMPLEX_UNARY(name) float _Complex name##f(float _Complex); double _Complex name(double _Complex); long double _Complex name##l(long double _Complex);
#define COMPLEX_REAL(name) float name##f(float _Complex); double name(double _Complex); long double name##l(long double _Complex);
COMPLEX_UNARY(cacos) COMPLEX_UNARY(cacosh) COMPLEX_UNARY(casin) COMPLEX_UNARY(casinh)
COMPLEX_UNARY(catan) COMPLEX_UNARY(catanh) COMPLEX_UNARY(ccos) COMPLEX_UNARY(ccosh)
COMPLEX_UNARY(cexp) COMPLEX_UNARY(clog) COMPLEX_UNARY(csin) COMPLEX_UNARY(csinh)
COMPLEX_UNARY(csqrt) COMPLEX_UNARY(ctan) COMPLEX_UNARY(ctanh)
COMPLEX_UNARY(conj) COMPLEX_UNARY(cproj)
COMPLEX_REAL(cabs) COMPLEX_REAL(carg) COMPLEX_REAL(cimag) COMPLEX_REAL(creal)
float _Complex cpowf(float _Complex,float _Complex);
double _Complex cpow(double _Complex,double _Complex);
long double _Complex cpowl(long double _Complex,long double _Complex);
#undef COMPLEX_UNARY
#undef COMPLEX_REAL
