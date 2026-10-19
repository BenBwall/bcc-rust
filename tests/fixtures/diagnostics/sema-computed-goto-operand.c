int f(long k) {
    void *p = &&a;
    goto *k;
a:
    return 0;
}

int g(void *p) {
    if (p)
        goto *p;
    return 1;
}
