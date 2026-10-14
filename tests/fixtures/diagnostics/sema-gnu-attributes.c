/* GNU entity attributes: applicability, arguments and alias resolution. */
__attribute__((frobnicate)) int unknown;
static int internal __attribute__((weak));
typedef int weak_type __attribute__((weak));
int target = 1;
int defined __attribute__((alias("target")));
extern int dangling __attribute__((alias("missing")));
void function(void) {}
extern int crossed __attribute__((alias("function")));
extern int loop_a __attribute__((alias("loop_b")));
extern int loop_b __attribute__((alias("loop_a")));
void retyped(int) __attribute__((alias("function")));
__attribute__((constructor(50))) void reserved(void) {}
__attribute__((destructor(70000))) void too_large(void) {}
__attribute__((constructor)) int not_a_function;
void stdcall_here(void) __attribute__((stdcall));
void local(void) {
    int automatic __attribute__((section(".data")));
}
int after;
