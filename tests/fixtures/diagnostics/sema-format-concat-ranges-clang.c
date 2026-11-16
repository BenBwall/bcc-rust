int printf(const char *, ...);
#define FMT "%d"
#define PCT "%"
#define D "d"
#define INNER "%d"
#define OUTER INNER
#define TWO "%d" " %s"
#define BODY "%s " FMT
void f(long l, int a, char *s) {
    printf(FMT " %s", l, a);
    printf("%s " FMT, a, l);
    printf("%s " FMT " %s", s, l, a);
    printf(FMT " %s" FMT, l, a, l);
    printf(OUTER " %s", l, a);
    printf("%s " OUTER, a, l);
    printf(TWO " %d", l, a, l);
    printf(BODY " %d", a, l, l);
    printf("%" D, l);
    printf(PCT "d", l);
    printf(PCT D, l);
    printf("%" "l" "d", a);
    printf(FMT " %y", a);
    printf("%y " FMT, a);
}
void g(long l, int a) {
    printf("%"
    "d", l);
    printf("%"
    "l"
    "d", a);
    printf("%5"
      ".2"
        "s", a);
    printf("%"
    D, l);
    printf(PCT
    "d", l);
    printf("%" "l"
    "d", a);
    printf("%"
    "y", l);
    printf("%-"
    "*d", l, a);
}
#define ID(x) x
#define JOIN(a, b) a b
#define TWICE(a) a a
void h(long l, int a) {
    printf(ID("%" "d"), l);
    printf(JOIN("%", "d"), l);
    printf(JOIN("%" "d", " %s"), l, a);
    printf(JOIN(" %s", "%" "d"), a, l);
    printf(JOIN("%", "d" " %s"), l, a);
    printf(TWICE("%"), l);
}
#define CAT(a, b) a ## b
#define CAT3(a, b, c) a ## b ## c
#define VCAT(a, ...) __VA_ARGS__ ## a
#define EMPTY
void k(long l, int a) {
    printf(CAT("%" "d",), l);
    printf(CAT(,"%" "d"), l);
    printf(CAT("%" "d" EMP, TY), l);
    printf(CAT("%" "d" "%" "d",), a, l);
    printf(VCAT(, "%" "d"), l);
    printf(CAT3("%" "d",,), l);
    printf(CAT3(,"%" "d",), l);
    printf(CAT3(,,"%" "d"), l);
    printf(CAT3(EMP, TY "%" "d" EMP, TY), l);
    printf(CAT3("%" EMP, , TY "d"), l);
}
#define NIL()
#define CALL(f, a) f a
#define CALL2(f, a, b) f a b
#define PCALL(f, a, b) f a ## b
void m(long l) {
    printf(CALL(NIL, () "%" "d"), l);
    printf(PCALL(NIL, () "%" "d",), l);
    printf(PCALL(NIL, () "%" "d" EMP, TY), l);
    printf(CALL2(NIL, () "%", "d"), l);
}
#define V(a, b, ...) __VA_OPT__(a b)
#define VCAT2(a, b, ...) __VA_OPT__(a ## b)
#define VTWICE(a, ...) __VA_OPT__(a a)
#define V1(a, ...) __VA_OPT__(a)
#define VPCT(a, ...) __VA_OPT__("%" a)
#define VCALL(f, a, b, ...) f __VA_OPT__(() a b)
#define VCALL1(f, a, ...) f __VA_OPT__(() a)
void va_opt(long l) {
    printf(V("%", "d", 1), l);
    printf(V("%" "d", "", 1), l);
    printf(VCAT2("%" "d", , 1), l);
    printf(VTWICE("%", 1), l);
    printf(V1("%" "d", 1), l);
    printf(VPCT("d", 1), l);
    printf(VCALL(NIL, "%", "d", 1), l);
    printf(VCALL1(NIL, "%" "d", 1), l);
}
#define VL(x, a, b, ...) x ## __VA_OPT__(a b)
#define VR(x, a, b, ...) __VA_OPT__(a b) ## x
#define VLL(x, a, ...) x ## __VA_OPT__(a a)
#define VCH(x, y, a, b, ...) x ## __VA_OPT__(a b) ## y
#define VCHR(x, a, b, ...) __VA_OPT__(a b) ## x ## x
#define VARGS(x, a, ...) x ## __VA_OPT__(a __VA_ARGS__)
#define VPASTE(x, a, b, c, ...) x ## __VA_OPT__(a ## b c)
#define VNIL(f, x, a, b, ...) f x ## __VA_OPT__(() a b)
#define VLIT(x, ...) x ## __VA_OPT__("%" "d")
#define CATS(x, a, b) x ## a b
void va_opt_paste(long l) {
    printf(VL(, "%", "d", 1), l);
    printf(VR(, "%", "d", 1), l);
    printf(VLL(, "%", 1), l);
    printf(VCH(, , "%", "d", 1), l);
    printf(VCHR(, "%", "d", 1), l);
    printf(VARGS(, "%", "d"), l);
    printf(VPASTE(, , "%", "d", 1), l);
    printf(VR(TY, "%", "d" EMP, 1), l);
    printf(VNIL(NIL, , "%", "d", 1), l);
    printf(VL(, "%" "d", "", 1), l);
    printf(VR(, "%" "d", , 1), l);
    printf(VL(u8, "%", "d", 1), l);
    printf(VLIT(u8, 1), l);
    printf(CATS(u8, "%", "d"), l);
    printf(CAT(u8, "%d"), l);
}
