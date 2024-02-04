#define PASTE(X, Y) X##Y

PASTE(+, +)
PASTE(1, 2)
PASTE(=, =)
PASTE(+, =)
PASTE(3, )
PASTE(, 4)

PASTE(A B, C D)

#define X 1##2

X
