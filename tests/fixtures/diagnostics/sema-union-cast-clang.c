/* GNU's cast to a union type: the target is a union, and the operand has
   the type of a member. A cast to a pointer to a union, as Lua's
   `cast(union GCUnion *, o)`, is an ordinary pointer conversion. */
union U { int i; double d; const char *s; };
typedef union U UT;
typedef const union U CUT;
struct S { int x; };
union W { struct S s; long l; };
union U *pointers(void *p, union U **pp) {
    union U **again = (union U **)pp;
    (void)(UT *)p;
    (void)(const union U *)p;
    return again ? *again : (union U *)p;
}
void values(int i, double d, union U u, struct S st, const int ci, short sh) {
    union U a = (union U)i;
    UT b = (UT)d;
    union U c = (const union U)ci;
    union U e = (CUT)i;
    union W w = (union W)st;
    union U same = (union U)u;
    union U bad = (union U)sh;
    (void)a; (void)b; (void)c; (void)e; (void)w; (void)same; (void)bad;
}
/* `__extension__` silences both reports, on its operand or on a whole
   declaration or function definition, as in GCC and Clang. */
#define MAKE(e) (__extension__ (union U)(e))
void quiet(int i, union U u, struct S st) {
    union U a = __extension__ (union U)i;
    __extension__ union U b = (union U)i;
    union U c = MAKE(i);
    union U same = __extension__ (union U)u;
    union W w = __extension__ ({ (union W)st; });
    union U loud = (union U)i;
    (void)a; (void)b; (void)c; (void)same; (void)w; (void)loud;
}
__extension__ int quiet_definition(int i) { return ((union U)i).i; }
__extension__ int quiet_size = sizeof((union U)1);
int loud_size = sizeof((union U)1);
int after;
