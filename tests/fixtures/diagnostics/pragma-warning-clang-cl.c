// clang-cl's warnings have no numbers, so it applies none; `push` and `pop`
// are its diagnostic pragmas', which clang-cl also reads.
#pragma warning(push, 3)
#pragma clang diagnostic warning "-Wimplicit-function-declaration"
#pragma warning(disable: 4013)
int f1(void) { return g1(); }
#pragma warning(pop)
int f2(void) { return g2(); }
#pragma warning
#pragma warning(bogus)
#pragma warning(disable 4013)
#pragma warning(push, 5)
#pragma warning(pop) junk
#pragma warning(pop)
