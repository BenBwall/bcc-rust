void f(void) { register __thread int a; }
_Thread_local typedef int T;
_Thread_local __thread int b;
static __thread _Thread_local int c;
constexpr thread_local int d = 1;
int following;
