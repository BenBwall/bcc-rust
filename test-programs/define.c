#define FOO() 3
#define M(...) #__VA_ARGS__
#define BAR(X, Y) X + Y
#define BAZ(...) __VA_ARGS__
#define C )
#define FOO( C 1

int x = FOO();
FOO()
BAR(5, 7)
M(1 2 3, 4, FOO())
BAZ(1, 2, 3, FOO())
