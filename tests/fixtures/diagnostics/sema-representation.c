int a __attribute__((aligned(3)));
typedef float bad __attribute__((mode(QI)));
_Alignas(1) int b;
typedef union {int n;} U;
void f(void) { U u=(U)1.0; }
int ranges[2] = {[2 ... 1]=1};
int following;
