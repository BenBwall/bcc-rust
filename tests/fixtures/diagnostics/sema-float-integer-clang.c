static int scaled = 1e10;
int negative = -1e10;
unsigned to_unsigned = -1.0;
unsigned truncated = -0.5;
signed char narrow[] = {300.0, -300.0};
int specials[] = {__builtin_nan(""), __builtin_inf()};
int from_float = 3e9f;
int from_long_double = 1e10L;
int inexact = 2147483647.5;
_Bool truth = 1e300;
int cast = (int)1e10;
int halved[] = {(int)-1e10 / 2};
struct fields { unsigned a : 4; int b : 3; } fields = {100.0, -100.0};
struct casts { unsigned a : 4; } casts = {(unsigned)1e10};
void f(void) {
    static int local = 1e10;
    int automatic = 1e10;
    automatic = 1.5;
    (void)local;
    (void)automatic;
}
int compared = (int)1e10 == 2147483647;
enum small { SMALL } enumerated = 1.5;
_Atomic int atomic = 2.5;
int following_valid;
