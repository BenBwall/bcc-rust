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
