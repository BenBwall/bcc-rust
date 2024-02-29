#define TRIPLE_PASTE(X, Y, Z) X ## Y ## Z

#define QUADRUPLE_PASTE(X, Y, Z, I, J) X ## Y ## Z ## I ## J

#define WIDE_STRING(X) L ## #X

TRIPLE_PASTE(A, B, C)

QUADRUPLE_PASTE(0x, 1, 2, 3, 4)

WIDE_STRING("ABC DEF")
