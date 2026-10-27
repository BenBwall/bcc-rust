/* GCC's __builtin_setjmp and __builtin_longjmp (GCC manual, "Nonlocal
   Gotos"): a five-word buffer, 1 as the second return's value, and no
   saved registers, so the compiler keeps every value live across the
   setjmp where the second return finds it. The program prints a transcript
   that GCC and Clang print too; it checks only values C99 §7.13.2.1p3
   determines (volatile objects and objects unchanged since the setjmp). */

int printf(const char *, ...);

#define NOINLINE __attribute__((noinline))

static void *buffer[5];
static int failures;

static void check(int ok, const char *what) {
    if (!ok) {
        failures++;
        printf("FAIL %s\n", what);
    }
}

/* A longjmp three calls below the setjmp. */
static NOINLINE void jump(void **target) { __builtin_longjmp(target, 1); }
static NOINLINE void middle(void **target, int depth) {
    if (depth > 0)
        middle(target, depth - 1);
    jump(target);
}

/* Values live across the setjmp in every register class: integers,
   pointers and doubles, more than there are registers, none changed. */
static NOINLINE int many_values(int seed) {
    int a = seed + 1, b = seed * 3, c = seed ^ 0x55, d = seed - 7;
    int e = seed * seed, f = seed << 3, g = ~seed, h = seed / 3 + 11;
    long i = (long)seed * 100003, j = seed + 0x10000000L;
    double x = seed * 0.5, y = seed + 0.25, z = 1.0 / (seed + 1);
    const char *s = "text" + (seed & 3);
    volatile int phase = 0;
    if (__builtin_setjmp(buffer) == 0) {
        phase = 1;
        middle(buffer, 2);
        return -1;
    }
    check(phase == 1, "volatile phase after the jump");
    check(a == seed + 1 && b == seed * 3 && c == (seed ^ 0x55) && d == seed - 7,
          "integers a-d");
    check(e == seed * seed && f == seed << 3 && g == ~seed && h == seed / 3 + 11,
          "integers e-h");
    check(i == (long)seed * 100003 && j == seed + 0x10000000L, "longs");
    check(x == seed * 0.5 && y == seed + 0.25 && z == 1.0 / (seed + 1), "doubles");
    check(s[0] == "text"[seed & 3], "pointer");
    return a + b + c + d + e + f + g + h + (int)(i + j) + (int)(x + y + z) + s[0];
}

/* The caller's own values must survive a call whose callee returns a
   second time: the callee saves and restores every callee-saved register. */
static NOINLINE int caller_values(int seed) {
    int p = seed * 7, q = seed + 13, r = seed * seed - 1, t = seed | 0x100;
    long u = seed * 1000L, v = -seed;
    int total = 0;
    for (int k = 0; k < 3; k++)
        total += many_values(seed + k) + p + q + r + t + (int)(u + v);
    check(p == seed * 7 && q == seed + 13 && r == seed * seed - 1 && t == (seed | 0x100),
          "caller integers");
    check(u == seed * 1000L && v == -seed, "caller longs");
    return total;
}

/* A loop of repeated setjmp and longjmp; the count is volatile. */
static NOINLINE int repeated(int rounds) {
    volatile int count = 0;
    void *local[5];
    while (__builtin_setjmp(local) != 0 || count == 0) {
        if (++count >= rounds)
            break;
        jump(local);
    }
    return count;
}

/* A setjmp inside a loop body: each iteration saves a new resume point. */
static NOINLINE int loop_body(int rounds) {
    volatile int sum = 0;
    for (int k = 0; k < rounds; k++) {
        void *local[5];
        if (__builtin_setjmp(local) == 0) {
            sum += k;
            jump(local);
        }
        sum += 1000;
    }
    return sum;
}

/* Variable length arrays and alloca around the setjmp: the frame's own
   objects keep their contents, and objects allocated after the setjmp in
   newer frames are released by the jump. */
static NOINLINE void allocate_then_jump(void **target, int n) {
    char *scratch = __builtin_alloca(n * 64);
    for (int k = 0; k < n * 64; k++)
        scratch[k] = (char)k;
    int vla[n * 8];
    vla[0] = scratch[1];
    if (vla[0] == 1)
        __builtin_longjmp(target, 1);
}

static NOINLINE int arrays(int n) {
    int vla[n];
    char *block = __builtin_alloca(n * 4);
    for (int k = 0; k < n; k++) {
        vla[k] = k * k;
        block[k] = (char)(k + 1);
    }
    void *local[5];
    volatile int rounds = 0;
    if (__builtin_setjmp(local) != 0) {
        if (++rounds < 50)
            allocate_then_jump(local, n);
        int sum = 0;
        for (int k = 0; k < n; k++)
            sum += vla[k] + block[k];
        return sum;
    }
    int later[n + 3];
    later[0] = n;
    allocate_then_jump(local, later[0]);
    return -1;
}

/* A longjmp through a copy of the buffer, as GCC's pr64242 does. */
static NOINLINE void jump_from_copy(void **original) {
    void *copy[5];
    __builtin_memcpy(copy, original, sizeof copy);
    __builtin_memset(original, 0, 5 * sizeof(void *));
    __builtin_longjmp(copy, 1);
}

static NOINLINE int copied(void) {
    void *local[5];
    volatile int marker = 3;
    if (__builtin_setjmp(local))
        return marker * 2;
    marker = 21;
    jump_from_copy(local);
    return -1;
}

/* Nested handlers: an inner setjmp in a callee, then a jump to the outer
   one from below it. */
static void *outer_buffer[5];

static NOINLINE int inner_handler(int depth) {
    void *inner[5];
    volatile int stage = 0;
    if (__builtin_setjmp(inner) == 0) {
        stage = 1;
        jump(inner);
    }
    check(stage == 1, "inner handler stage");
    if (depth > 0)
        middle(outer_buffer, depth);
    return stage;
}

static NOINLINE int nested(void) {
    volatile int reached = 0;
    if (__builtin_setjmp(outer_buffer) == 0) {
        reached = inner_handler(3);
        return -1;
    }
    return reached + 40;
}

int main(void) {
    printf("many: %d\n", many_values(5));
    printf("caller: %d\n", caller_values(9));
    printf("repeated: %d\n", repeated(1000));
    printf("loop body: %d\n", loop_body(10));
    printf("arrays: %d\n", arrays(17));
    printf("copied: %d\n", copied());
    printf("nested: %d\n", nested());
    printf("value: %d\n", __builtin_setjmp(buffer) + 7);
    printf("failures: %d\n", failures);
    return failures;
}
