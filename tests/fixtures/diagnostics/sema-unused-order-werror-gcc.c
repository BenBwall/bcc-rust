/* A warning -Werror makes an error is not one of GCC's errors: the unused
   typedefs after it are still reported. */
void first(void) {
    typedef int T;
    int x;
}

void second(void) {
    typedef int U;
}
