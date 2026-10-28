#include <pointer-casts.h>
void f(int i, void *p) {
    (void)TO_POINTER(i);
    (void)TO_INT(p);
    (void)(void *)i;
}
int following_valid;
