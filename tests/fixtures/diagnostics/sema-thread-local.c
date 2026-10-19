_Thread_local int defined = 1;
int plain;
_Thread_local int plain;
__thread int tls;
extern int tls;
void f(void) {
    __thread int automatic;
    static _Thread_local int counted;
    extern _Thread_local int defined;
}
_Thread_local void function(void);
void parameter(_Thread_local int p);
static int *address = &defined;
struct S { __thread int member; };
int following;
