// Compound literals in the initializers of static objects. The forms both
// GCC and Clang accept come first: a literal of the object's type, inside
// the wrappers GCC's folder removes (casts and ?: only without braces), and
// the addresses of file-scope literals. Clang's evaluator also reads a
// scalar literal's value inside any constant expression, at file and block
// scope.
int printf(const char *, ...);

struct pair { int a, b; };
enum colour { RED, GREEN, BLUE };
static int g = 7;

static double scaled = (double)((double){0x1p20});
static int round_trip = (int)(long)(unsigned)(int){-3};
static float widened = (float)(long double)(double)(float){1.5f};
static _Bool truth = (_Bool)(int)(_Bool){2};
static enum colour colour = (enum colour)(int)(enum colour){BLUE};
static int *pointer = (int *)(void *)(int *){&g};
static int dereferenced = *&(int){11};
static int selected = 1 ? (int){12} : 0;
static int skipped = 0 ? g : (int){13};
static int extension = __extension__ (const int){14};
static _Complex double complex_value = (_Complex double)(_Complex double){2.0};
static struct pair copied = (struct pair)(struct pair){15, 16};
static struct pair through = *&(struct pair){17, 18};
static struct pair chosen = 1.0 ? (struct pair){19, 20} : (struct pair){0, 0};
static int elements[3] = { (int){21}, ((int){22}), *&(int){23} };
static struct { int x : 4; unsigned y : 3; } bits = { (int){-2}, (unsigned){5} };
// Inside braces a literal of another arithmetic type converts its value.
static double converted_elements[2] = { (int){1}, (char){300} };
static int truncated_element[1] = { (double){2.5} };
static __bf16 brain_element[1] = { *&(_Float16){0.1} };
static struct { int x : 4; } converted_bits = { (double){-2.5} };

// The half-precision types, through the conversions GCC folds away: it
// keeps (_Float16)(__bf16) unrounded, as both are 16 bits wide, and its
// literal's initializer keeps float's excess until the literal's type.
static __bf16 brain = (__bf16){0.1};
static _Float16 half = (_Float16){0.1};
static _Float16 half_excess = (_Float16){0.1f16 * 3 + 0.2f16};
static __bf16 brain_excess = (__bf16){(__bf16)0.1 * 3};
static _Complex _Float16 complex_half = (_Complex _Float16){1.5};
static _Float16 half_pair = (_Float16)(__bf16)(_Float16){0.1};
static __bf16 brain_through_float = (__bf16)(float)(__bf16){0.3};
static int generic = _Generic(0, int: (int){56});
static double chosen_value = __builtin_choose_expr(1, (double){57.5}, 0);
static _Float16 generic_half = _Generic(0, int: (_Float16){0.7});

// File-scope literals have static storage, so their addresses are address
// constants (C99 6.5.2.5p6 and 6.6p9).
static int *scalar_address = &(int){24};
static int *array = (int[]){25, 26, 27};
static int *element = &(int[]){28, 29, 30}[1];
static int *past = (int[]){31, 32, 33} + 2;
static int *member = &(struct pair){34, 35}.b;
static const char *text = (const char[]){"literal"};
static long as_integer = (long)&(int){36};
static double *real_address = &(double){36.5};

#ifdef __clang__
static int sum = (int){3} + 1;
static double product = 2 * (double){1.25};
static int truncated = (int)(double){37.9};
static double converted = (int){38};
static int negated = -(int){39};
static int narrowed = (int)(char)(int){300};
static float from_int = (float)(int){40};
static int conditional = (int){1} ? 41 : 0;
static int folded[] = { (int){42} * 2, (short){43}, (int)(int){44} };
enum { FOLDED = (int){45} + 1 };
static char bound[(int){3}];
static _Float16 half_product = (_Float16){0.1} * 3;
static float mixed_halves = (_Float16){0.1} + (__bf16){0.1};
static __fp16 storage_half = (__fp16){0.1};
static float storage_product = (__fp16){0.1} * 3;
static int empty_braces = (int){} + 1;
#endif

static float halves(void) {
    _Float16 product = (_Float16){0.1} * 3;
    __bf16 brain_product = (__bf16){0.1} * 3;
    float mixed = (_Float16){0.1} * (_Float16){3} + (__bf16){0.2};
    _Float16 *object = &(_Float16){0.5};
    *object *= 3;
    return (float)product + (float)brain_product + mixed + (float)*object;
}

static int local(void) {
    static int kept = (int){50};
    static double cast = (double)((double){51.5});
    static struct pair whole = (struct pair){52, 53};
#ifdef __clang__
    static int plus = (int){54} + 1;
#else
    static int plus = 55;
#endif
    // A literal whose value folds is still an object of its own.
    double *real = &(double){1.5};
    *real += (double){0.25};
    _Complex double *complex = &(_Complex double){2.0};
    *complex *= 2;
    (float){1} = 2;
    return kept + (int)(cast * 2) + whole.a + whole.b + plus + (int)(*real * 4)
           + (int)__real__ *complex;
}

int main(void) {
    printf("%a %d %a %d %d %d\n", scaled, round_trip, (double)widened, truth,
           colour, *pointer);
    printf("%d %d %d %d\n", dereferenced, selected, skipped, extension);
    printf("%a %a\n", __real__ complex_value, __imag__ complex_value);
    printf("%d %d %d %d %d %d\n", copied.a, copied.b, through.a, through.b,
           chosen.a, chosen.b);
    printf("%d %d %d %d %d\n", elements[0], elements[1], elements[2], bits.x,
           bits.y);
    printf("%d %d %d %d %d %d %s %d %a\n", *scalar_address, array[0],
           array[2], *element, *past, *member, text, *(int *)as_integer,
           *real_address);
#ifdef __clang__
    printf("%d %a %d %a %d %d %a %d\n", sum, product, truncated, converted,
           negated, narrowed, (double)from_int, conditional);
    printf("%d %d %d %d %d\n", folded[0], folded[1], folded[2], FOLDED,
           (int)sizeof bound);
#endif
    printf("%a %a %a %a %a %a\n", (double)brain, (double)half,
           (double)half_excess, (double)brain_excess,
           (double)__real__ complex_half, (double)half_pair);
    printf("%a %d %a %a %a\n", (double)brain_through_float, generic,
           chosen_value, (double)generic_half, (double)halves());
#ifdef __clang__
    printf("%a %a %a %a %d\n", (double)half_product, (double)mixed_halves,
           (double)storage_half, (double)storage_product, empty_braces);
#endif
    printf("%a %a %d %a %d\n", converted_elements[0], converted_elements[1],
           truncated_element[0], (double)brain_element[0], converted_bits.x);
    // GCC's __builtin_constant_p reads no compound literal that is
    // evaluated; Clang's reads a scalar one.
    printf("%d %d %d %d %d %d %d\n", __builtin_constant_p((int){1}),
           __builtin_constant_p((int){1} ? 2 : 3),
           __builtin_constant_p(_Generic((int){1}, int: 1, default: 2)),
           __builtin_constant_p(0 && (int){1}),
           __builtin_constant_p(1 ? 2 : (int){3}),
           __builtin_constant_p(1 || (int){1}),
           __builtin_constant_p(__builtin_choose_expr(1, 4, (int){3})));
    printf("%d %d %d\n", __builtin_constant_p(1 ?: (int){1}),
           __builtin_constant_p((void *)0 ? (int){1} : 2),
           __builtin_constant_p(&g ? 2 : (int){1}));
    printf("%d\n", local());
    return 0;
}
