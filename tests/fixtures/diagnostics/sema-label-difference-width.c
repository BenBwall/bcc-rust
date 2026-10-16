int f(void) {
    static int ok = &&b - &&a;
    static short narrow = &&b - &&a;
a:
b:
    return ok + narrow;
}
