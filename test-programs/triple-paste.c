#define TRIPLE_PASTE(X, Y, Z) X ## Y ## Z

#define WIDE_STRING(X) L ## #X

TRIPLE_PASTE(A, B, C)

WIDE_STRING("ABC DEF")
