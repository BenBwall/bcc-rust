# 1 "sema-shadow-system.h" 1 3
extern int optind;
typedef int systype;
int sysfun(void);
# 5 "sema-shadow-system-clang.c" 2
void f(void) { int optind = 0; (void)optind; { enum { optind = 2 }; } }
void g(void) { typedef int systype; int sysfun = 0; (void)sysfun; }
void h(int optind) { (void)optind; }
