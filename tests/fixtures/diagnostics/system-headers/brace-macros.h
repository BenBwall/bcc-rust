/* A system header's initializers and macros for the -Wmissing-braces
   fixtures: Clang warns about neither. */
struct sys_pair { int x, y; };
struct sys_outer { struct sys_pair pair; int z; };
static struct sys_outer sys_object = {1, 2, 3};
#define SYS_INIT {1, 2, 3}
#define SYS_PAIR 1, 2
