/* errno as the host's C library functions set it, and its messages: what
   strerror, both forms of strerror_r and perror say for every code; the
   errno of failing conversions, allocations, stream and file operations and
   math functions in each precision, binary128 included. The interpreter must print what the
   program built against glibc prints, to stdout and to stderr.
   Declarations are written out so that no system header is needed. */
typedef struct _IO_FILE FILE;
typedef unsigned long size_t;
extern FILE *stdout, *stderr;
int *__errno_location(void);
#define errno (*__errno_location())
int printf(const char *, ...);
int fprintf(FILE *, const char *, ...);
int fflush(FILE *);
char *strerror(int);
char *strerror_r(int, char *, size_t);
int __xpg_strerror_r(int, char *, size_t);
void perror(const char *);
long strtol(const char *, char **, int);
unsigned long strtoul(const char *, char **, int);
long long strtoll(const char *, char **, int);
unsigned long long strtoull(const char *, char **, int);
long __isoc23_strtol(const char *, char **, int);
int atoi(const char *);
double strtod(const char *, char **);
float strtof(const char *, char **);
long double strtold(const char *, char **);
double atof(const char *);
void *malloc(size_t);
void *calloc(size_t, size_t);
void *realloc(void *, size_t);
void free(void *);
FILE *fopen(const char *, const char *);
int fclose(FILE *);
int fputc(int, FILE *);
int fgetc(FILE *);
int fseek(FILE *, long, int);
long ftell(FILE *);
int ferror(FILE *);
int remove(const char *);
char *tmpnam(char *);
int sscanf(const char *, const char *, ...) __asm__("__isoc99_sscanf");
int sscanf23(const char *, const char *, ...) __asm__("__isoc23_sscanf");
double sqrt(double), log(double), log10(double), exp(double), pow(double, double),
    acos(double), atanh(double), fmod(double, double), ldexp(double, int),
    lgamma(double), tgamma(double), cosh(double), hypot(double, double), log1p(double);
float sqrtf(float), logf(float), expf(float), powf(float, float);
long double sqrtl(long double), fmodl(long double, long double),
    remainderl(long double, long double), ldexpl(long double, int),
    scalbnl(long double, int), scalblnl(long double, long),
    nextafterl(long double, long double), fdiml(long double, long double),
    floorl(long double), logl(long double), log10l(long double),
    log2l(long double), log1pl(long double), expl(long double),
    exp2l(long double), expm1l(long double), powl(long double, long double),
    acosl(long double), asinl(long double), atanhl(long double),
    acoshl(long double), sinl(long double), cosl(long double), tanl(long double),
    coshl(long double), sinhl(long double), erfcl(long double),
    atan2l(long double, long double), hypotl(long double, long double),
    atanl(long double);
int ilogbl(long double);
double nexttoward(double, long double);
float nexttowardf(float, long double);

/* Prints the call, its result as text, and errno, then clears errno. */
typedef __float128 quad;
quad strtof128(const char *, char **);
quad sqrtf128(quad), fmodf128(quad, quad), ldexpf128(quad, int), scalbnf128(quad, int),
    fdimf128(quad, quad), rintf128(quad), fmaxf128(quad, quad), frexpf128(quad, int *);
static volatile quad quad_zero = 0, quad_one = 1;

/* As SHOW, for a binary128 call, whose value printf cannot print. */
#define SHOW_QUAD(call)                                                        \
  do {                                                                         \
    errno = 0;                                                                 \
    volatile quad value = call;                                                \
    (void)value;                                                               \
    printf("%-34s errno %d\n", #call, errno);                                  \
  } while (0)

#define SHOW(format, call)                                                     \
  do {                                                                         \
    errno = 0;                                                                 \
    printf("%-34s " format " errno %d\n", #call, call, errno);                \
  } while (0)

static char text[64];
/* NaNs computed at run time, as the processor makes them: compilers fold a
   constant 0.0L / 0.0L to NaNs of different signs. */
static volatile long double zero = 0.0L;

int main(void) {
  printf("initial errno %d\n", errno);
  for (int code = 0; code <= 134; code++)
    printf("%d %s\n", code, strerror(code));
  printf("%s\n", strerror(-1));
  printf("%s\n", strerror(4000));

  /* The GNU form returns its message, the XSI form copies it. */
  char small[8];
  char *gnu = strerror_r(34, small, sizeof small);
  printf("gnu known [%s] in buffer %d\n", gnu, gnu == small);
  gnu = strerror_r(9999, small, sizeof small);
  printf("gnu unknown [%s] in buffer %d\n", gnu, gnu == small);
  int xsi = __xpg_strerror_r(34, small, sizeof small);
  printf("xsi known %d [%s]\n", xsi, small);
  xsi = __xpg_strerror_r(2, text, sizeof text);
  printf("xsi fits %d [%s]\n", xsi, text);
  xsi = __xpg_strerror_r(9999, small, sizeof small);
  printf("xsi unknown %d [%s]\n", xsi, small);
  printf("xsi empty %d\n", __xpg_strerror_r(34, small, 0));

  fflush(stdout);
  errno = 33;
  perror("prefix");
  errno = 9999;
  perror("");
  errno = 2;
  perror(0);
  printf("perror keeps errno %d\n", errno);

  char *end;
  SHOW("%ld", strtol("99999999999999999999", 0, 10));
  SHOW("%ld", strtol("-99999999999999999999", 0, 10));
  SHOW("%lu", strtoul("99999999999999999999", 0, 10));
  SHOW("%lu", strtoul("-1", 0, 10));
  SHOW("%lld", strtoll("9223372036854775808", 0, 0));
  SHOW("%llu", strtoull("0x10000000000000000", 0, 16));
  SHOW("%ld", strtol("12", 0, 1));
  SHOW("%ld", strtol("12", 0, 37));
  SHOW("%ld", __isoc23_strtol("0b101", 0, 0));
  SHOW("%ld", __isoc23_strtol("0b101", 0, 2));
  SHOW("%ld", strtol("0b101", 0, 0));
  SHOW("%d", atoi("99999999999999999999"));
  SHOW("%g", strtod("1e999", &end));
  SHOW("%g", strtod("-1e999", &end));
  SHOW("%g", strtod("1e-999", &end));
  SHOW("%a", strtod("0x1p-1074", &end));
  SHOW("%a", strtod("0x1p-1080", &end));
  SHOW("%g", strtod("  42.5e1xyz", &end));
  printf("end [%s]\n", end);
  SHOW("%g", strtod("1e", &end));
  printf("end [%s]\n", end);
  SHOW("%g", strtod("infinity!", &end));
  printf("end [%s]\n", end);
  SHOW("%g", strtod("nan(12)x", &end));
  printf("end [%s]\n", end);
  SHOW("%g", strtod("junk", &end));
  printf("end [%s]\n", end);
  SHOW("%g", (double)strtof("1e39", &end));
  SHOW("%g", (double)strtof("1e-50", &end));
  SHOW("%g", (double)strtof("3.25", &end));
  SHOW("%Lg", strtold("1e5000", &end));
  SHOW("%Lg", strtold("1e-5000", &end));
  SHOW("%La", strtold("0x1.8p-16400", &end));
  SHOW("%.20Lg", strtold("1.10000000000000000001", &end));
  SHOW("%g", atof("  -2.5e3"));

  SHOW("%p", malloc((size_t)-1));
  SHOW("%p", calloc((size_t)1 << 40, (size_t)1 << 40));
  char *grown = realloc(0, 4);
  grown[0] = 'a', grown[1] = 'b', grown[2] = 'c', grown[3] = 0;
  grown = realloc(grown, 1000);
  printf("realloc keeps [%s]\n", grown);
  SHOW("%p", realloc(grown, (size_t)-1));
  printf("realloc failure keeps [%s]\n", grown);
  SHOW("%p", realloc(grown, 0));

  SHOW("%p", (void *)fopen("/nonexistent-bcc-dir/file", "r"));
  char *name = tmpnam(0);
  FILE *file = fopen(name, "w");
  SHOW("%p", (void *)fopen(name, "q"));
  SHOW("%d", fgetc(file));
  printf("ferror %d\n", ferror(file));
  SHOW("%d", fseek(file, 0, 7));
  SHOW("%d", fseek(file, -5, 0));
  SHOW("%d", fseek(file, -5, 1));
  fclose(file);
  file = fopen(name, "r");
  SHOW("%d", fputc('x', file));
  printf("ferror %d\n", ferror(file));
  fclose(file);
  SHOW("%d", remove(name));
  SHOW("%d", remove(name));

  long value;
  int number;
  SHOW("%d", sscanf("99999999999999999999", "%ld", &value));
  printf("value %ld\n", value);
  SHOW("%d", sscanf("-99999999999999999999", "%d", &number));
  printf("number %d\n", number);
  SHOW("%d", sscanf("4294967296", "%u", (unsigned *)&number));
  printf("number %d\n", number);
  int binary, prefixed;
  SHOW("%d", sscanf23("101 0b11 0B1x", "%b %i %b", &binary, &prefixed, &number));
  printf("C23 %d %d %d\n", binary, prefixed, number);
  SHOW("%d", sscanf("0b11", "%i", &prefixed));
  printf("C99 %d\n", prefixed);

  SHOW("%g", sqrt(-1.0));
  SHOW("%g", log(0.0));
  SHOW("%g", log(-1.0));
  SHOW("%g", log10(0.0));
  SHOW("%g", exp(1000.0));
  SHOW("%g", exp(-1000.0));
  SHOW("%g", pow(0.0, -1.0));
  SHOW("%g", pow(-8.0, 1.0 / 3));
  SHOW("%g", pow(10.0, 400.0));
  SHOW("%g", pow(10.0, -400.0));
  SHOW("%g", acos(2.0));
  SHOW("%g", atanh(1.0));
  SHOW("%g", fmod(1.0, 0.0));
  SHOW("%g", ldexp(1.0, 5000));
  SHOW("%g", lgamma(0.0));
  SHOW("%g", tgamma(-1.0));
  SHOW("%g", tgamma(200.0));
  SHOW("%g", cosh(1000.0));
  SHOW("%g", hypot(1e308, 1e308));
  SHOW("%g", log1p(-1.0));
  SHOW("%g", log1p(-2.0));
  SHOW("%g", sqrt(4.0));
  SHOW("%g", (double)sqrtf(-1.0f));
  SHOW("%g", (double)logf(0.0f));
  SHOW("%g", (double)expf(100.0f));
  SHOW("%g", (double)powf(0.0f, -2.0f));
  SHOW("%Lg", sqrtl(-1.0L));
  SHOW("%Lg", sqrtl(2.25L));
  SHOW("%Lg", fmodl(1.0L, 0.0L));
  SHOW("%Lg", fmodl(1.0L / 0.0L, 2.0L));
  SHOW("%Lg", fmodl(zero / zero, 0.0L));
  SHOW("%Lg", remainderl(1.0L, 0.0L));
  SHOW("%Lg", remainderl(5.0L, 3.0L));
  SHOW("%Lg", ldexpl(1.0L, 100000));
  SHOW("%Lg", ldexpl(1.0L, -100000));
  SHOW("%Lg", ldexpl(0.0L, 100000));
  SHOW("%Lg", scalbnl(1.0L, 100000));
  SHOW("%Lg", scalblnl(1.0L, -100000L));
  SHOW("%d", ilogbl(0.0L));
  SHOW("%d", ilogbl(1.0L / 0.0L));
  SHOW("%d", ilogbl(8.0L));
  SHOW("%Lg", nextafterl(0.0L, 1.0L));
  SHOW("%Lg", nextafterl(0x1p-16445L, 0.0L));
  SHOW("%Lg", nextafterl(1.0L, 2.0L));
  SHOW("%Lg", nextafterl(0x1.fffffffffffffffep16383L, 1.0L / 0.0L));
  SHOW("%a", nexttoward(0x1p-1022, 0.0L));
  SHOW("%a", nexttoward(0.0, 1.0L));
  SHOW("%a", (double)nexttowardf(0x1p-126f, 0.0L));
  SHOW("%Lg", fdiml(0x1p16383L, -0x1p16383L));
  SHOW("%Lg", floorl(-2.5L));
  SHOW("%Lg", logl(0.0L));
  SHOW("%Lg", logl(-1.0L));
  SHOW("%Lg", log10l(0.0L));
  SHOW("%Lg", log2l(-0.5L));
  SHOW("%Lg", log1pl(-1.0L));
  SHOW("%Lg", log1pl(-3.0L));
  SHOW("%Lg", expl(20000.0L));
  SHOW("%Lg", expl(-20000.0L));
  SHOW("%Lg", expl(1.0L));
  SHOW("%Lg", exp2l(20000.0L));
  SHOW("%Lg", expm1l(20000.0L));
  SHOW("%Lg", expm1l(-20000.0L));
  SHOW("%Lg", powl(0.0L, -3.0L));
  SHOW("%Lg", powl(-8.0L, 0.5L));
  SHOW("%Lg", powl(10.0L, 6000.0L));
  SHOW("%Lg", powl(10.0L, -6000.0L));
  SHOW("%Lg", powl(2.0L, 10.0L));
  SHOW("%Lg", acosl(2.0L));
  SHOW("%Lg", asinl(-1.5L));
  SHOW("%Lg", atanhl(1.0L));
  SHOW("%Lg", atanhl(2.0L));
  SHOW("%Lg", acoshl(0.5L));
  SHOW("%Lg", sinl(1.0L / 0.0L));
  SHOW("%Lg", cosl(-1.0L / 0.0L));
  SHOW("%Lg", tanl(1.0L / 0.0L));
  SHOW("%Lg", sinl(1e4000L));
  SHOW("%Lg", coshl(20000.0L));
  SHOW("%Lg", sinhl(-20000.0L));
  SHOW("%Lg", erfcl(200.0L));
  SHOW("%Lg", erfcl(1.0L));
  SHOW("%Lg", atan2l(1e-4000L, 1e4000L));
  SHOW("%Lg", hypotl(1e4932L, 1e4932L));
  SHOW("%Lg", atanl(1.0L / 0.0L));
  /* The binary128 functions set errno as glibc's _Float128 wrappers do. */
  int exponent;
  SHOW_QUAD(sqrtf128(-quad_one));
  SHOW_QUAD(sqrtf128(-quad_zero));
  SHOW_QUAD(sqrtf128(-quad_one / quad_zero));
  SHOW_QUAD(sqrtf128(quad_zero / quad_zero));
  SHOW_QUAD(fmodf128(quad_one, quad_zero));
  SHOW_QUAD(fmodf128(quad_one / quad_zero, quad_one));
  SHOW_QUAD(fmodf128(quad_zero / quad_zero, quad_zero));
  SHOW_QUAD(ldexpf128(quad_one, 100000));
  SHOW_QUAD(ldexpf128(quad_one, -100000));
  SHOW_QUAD(ldexpf128(quad_one, -16400));
  SHOW_QUAD(ldexpf128(quad_zero, 100000));
  SHOW_QUAD(scalbnf128(quad_one, 100000));
  SHOW_QUAD(scalbnf128(quad_one, -16494));
  SHOW_QUAD(fdimf128(ldexpf128(quad_one, 16383), -ldexpf128(quad_one, 16383)));
  SHOW_QUAD(rintf128(ldexpf128(quad_one, 16383)));
  SHOW_QUAD(fmaxf128(quad_zero / quad_zero, quad_one));
  SHOW_QUAD(frexpf128(quad_one / quad_zero, &exponent));
  SHOW_QUAD(strtof128("1e99999", &end));
  SHOW_QUAD(strtof128("-1e99999", &end));
  SHOW_QUAD(strtof128("1e-99999", &end));
  SHOW_QUAD(strtof128("1e-4940", &end));
  SHOW_QUAD(strtof128("0x1p-16494", &end));
  SHOW_QUAD(strtof128("0x1p-16450", &end));
  SHOW_QUAD(strtof128("0x1.8p-16494", &end));
  SHOW_QUAD(strtof128("0x1p-16495", &end));
  SHOW_QUAD(strtof128("0x1.ffffffffffffffffffffffffffff8p16383", &end));
  SHOW_QUAD(strtof128("0x1.ffffffffffffffffffffffffffffp16383", &end));
  SHOW_QUAD(strtof128("3.3621031431120935062626778173217526e-4932", &end));
  SHOW_QUAD(strtof128("0e-99999", &end));
  SHOW_QUAD(strtof128("1.5", &end));
  return 0;
}
