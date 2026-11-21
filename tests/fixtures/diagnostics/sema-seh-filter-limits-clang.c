// Clang checks a comparison's limits as it finishes the full expression,
// which an __except filter never is, so the filter draws none of them; its
// pointer comparison is checked as the operator is built (Clang 23.1.5,
// --target=x86_64-pc-windows-msvc -fms-extensions -fsyntax-only -Wtype-limits
// -Wtautological-compare -Wtautological-type-limit-compare).
int work(int);

int filters(int p, unsigned u, signed char c, _Bool b) {
    __try {
        work(p);
    } __except (u < 0) {
    }
    __try {
        work(p);
    } __except (0 <= u) {
    }
    __try {
        work(p);
    } __except (c < 1000) {
    }
    __try {
        work(p);
    } __except (b == 2) {
    }
    __try {
        work(p);
    } __except (p <= 2147483647) {
    }
    __try {
        work(p);
    } __except (&p == 0) {
    }
    return 0;
}
