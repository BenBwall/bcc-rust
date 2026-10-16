#include <pointer-macros.h>
#define USER_SET(a) ((a) = &sys_u)
int *ip; int i;
void set(void) { SYS_SET(ip); }
void assign(void) { SYS_ASSIGN(ip, &sys_u); }
void user(void) { USER_SET(ip); }
int compare(void) { return SYS_COMPARE(ip, &sys_u); }
void incompatible(void) { float *fp; SYS_SET(fp); (void)fp; }
void integer(void) { SYS_ASSIGN(ip, i); }
int following_valid;
