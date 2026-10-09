void outer(void) {
    static int inner(void) { return 0; }
    extern int other(void) { return 1; }
    int after = inner() + other();
}
