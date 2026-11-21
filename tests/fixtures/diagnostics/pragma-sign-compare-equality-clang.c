/* bcc keeps cl's C4389 group unnamed: no pragma names it. */
#pragma GCC diagnostic warning "-Wsign-compare-equality"
#pragma clang diagnostic warning "-Wsign-compare-equality"
int f(int i, unsigned u) { return i == u; }
