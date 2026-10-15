/* Pointer constraints a system header commits itself are withheld, even the
   ones that default to errors; its macros expanded in user code are not. */
extern unsigned sys_u;
static inline void sys_sign(int *p) { p = &sys_u; (void)p; }
static inline void sys_incompatible(float *p) { p = &sys_u; (void)p; }
static inline int sys_compare(int *p, unsigned *q) { return p == q; }
static inline void sys_integer(int *p) { p = 5; (void)p; }
#define SYS_SET(a) ((a) = &sys_u)
#define SYS_ASSIGN(a, b) ((a) = (b))
#define SYS_COMPARE(a, b) ((a) == (b))
