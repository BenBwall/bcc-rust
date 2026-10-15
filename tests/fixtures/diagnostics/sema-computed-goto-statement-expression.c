int f(void) {
    void *p = &&in;
    goto *p;
    return ({ in: 1; });
}
