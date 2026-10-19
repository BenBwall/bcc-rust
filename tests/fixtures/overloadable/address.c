int o(int) __attribute__((overloadable));
int o(double) __attribute__((overloadable));
extern inline __attribute__((always_inline, gnu_inline, overloadable))
int o(int x) { return x + 1000; }

typedef int (*IntFn)(int);
typedef int (*DoubleFn)(double);

#if defined(AMBIGUOUS_ADDRESS)
/* Neither overload matches this target; a cast must not choose arbitrarily. */
void (*ambiguous_address)(void) = (void (*)(void))&o;
#elif defined(AMBIGUOUS_DECAY)
void (*ambiguous_decay)(void) = (void (*)(void))o;
#else
IntFn global_int = &o;
DoubleFn global_double = &o;
DoubleFn global_decay_double = o;

IntFn static_int(void) { static IntFn p = &o; return p; }
DoubleFn static_double(void) { static DoubleFn p = o; return p; }
IntFn automatic_int(void) { IntFn p = &o; return p; }
DoubleFn automatic_double(void) { DoubleFn p = &o; return p; }
IntFn assignment_int(void) { IntFn p; p = &o; return p; }
DoubleFn assignment_double(void) { DoubleFn p; p = &o; return p; }
IntFn cast_int(void) { return (IntFn)&o; }
DoubleFn cast_double(void) { return (DoubleFn)&o; }
int take_int(IntFn);
int take_double(DoubleFn);
int argument_int(void) { return take_int(&o); }
int argument_double(void) { return take_double(&o); }
#endif
