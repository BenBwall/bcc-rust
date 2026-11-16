// Constraint violations the compilers only warn about by default, and then
// compile: an initializer with too many elements (C99 §6.7.8p2) and a macro
// defined again differently (§6.10.3p2), which takes the new replacement:
// the program exits with 2. The glossary's "constraint" and the #define
// step.
#define N 1
#define N 2
int main(void) {
    int a[1] = {1, 2};
    return a[0] + N - 1;
}
