__attribute__((used)) __int128 wide[0];
__typeof__(wide) copy; __auto_type value=1; struct Empty {};
__asm__("nop"); int symbol __asm__("name");
union U { int i; }; int ranges[4]={[1 ... 3]=2};
int f(void) {
    __label__ L;
    int nested(int x) { return x; }
    __asm__ __volatile__ __inline__("" : [out] "=r"(value) : "r"(value) : "memory");
    __asm__ goto("" : : : : L);
    void *p=&&L; goto *p;
L: switch(1){case 1 ... 3:break;}
    union U u=(union U)1;
    struct S {int x;}; struct S s={x:1};
    int b=__builtin_va_arg(ap,int)+__builtin_offsetof(struct S,x);
    b+=__builtin_types_compatible_p(int,long)+__builtin_choose_expr(1,2,3);
    return ({ b+__real__ u.i+__imag__ u.i; }) ?: 2;
}
