/* C99 library declarations for a target-independent tgmath resource probe.
 * These are declarations, so the probe needs no host C library or linker. */
#define REAL_UNARY(name) float name##f(float); double name(double); long double name##l(long double);
#define REAL_BINARY(name) float name##f(float,float); double name(double,double); long double name##l(long double,long double);
#define REAL_ARGUMENT(name,result) result name##f(float); result name(double); result name##l(long double);
REAL_UNARY(acos) REAL_UNARY(acosh) REAL_UNARY(asin) REAL_UNARY(asinh)
REAL_UNARY(atan) REAL_UNARY(atanh) REAL_UNARY(cbrt) REAL_UNARY(ceil)
REAL_UNARY(cos) REAL_UNARY(cosh) REAL_UNARY(erf) REAL_UNARY(erfc)
REAL_UNARY(exp) REAL_UNARY(exp2) REAL_UNARY(expm1) REAL_UNARY(fabs)
REAL_UNARY(floor) REAL_UNARY(lgamma) REAL_UNARY(log) REAL_UNARY(log10)
REAL_UNARY(log1p) REAL_UNARY(log2) REAL_UNARY(logb) REAL_UNARY(nearbyint)
REAL_UNARY(rint) REAL_UNARY(round) REAL_UNARY(sin) REAL_UNARY(sinh)
REAL_UNARY(sqrt) REAL_UNARY(tan) REAL_UNARY(tanh) REAL_UNARY(tgamma)
REAL_UNARY(trunc)
REAL_BINARY(atan2) REAL_BINARY(copysign) REAL_BINARY(fdim) REAL_BINARY(fmax)
REAL_BINARY(fmin) REAL_BINARY(fmod) REAL_BINARY(hypot) REAL_BINARY(nextafter)
REAL_BINARY(pow) REAL_BINARY(remainder)
REAL_ARGUMENT(ilogb,int) REAL_ARGUMENT(lrint,long) REAL_ARGUMENT(lround,long)
REAL_ARGUMENT(llrint,long long) REAL_ARGUMENT(llround,long long)
float fmaf(float,float,float); double fma(double,double,double);
long double fmal(long double,long double,long double);
float frexpf(float,int*); double frexp(double,int*); long double frexpl(long double,int*);
float ldexpf(float,int); double ldexp(double,int); long double ldexpl(long double,int);
float scalbnf(float,int); double scalbn(double,int); long double scalbnl(long double,int);
float scalblnf(float,long); double scalbln(double,long); long double scalblnl(long double,long);
float nexttowardf(float,long double); double nexttoward(double,long double);
long double nexttowardl(long double,long double);
float remquof(float,float,int*); double remquo(double,double,int*);
long double remquol(long double,long double,int*);
#undef REAL_UNARY
#undef REAL_BINARY
#undef REAL_ARGUMENT
