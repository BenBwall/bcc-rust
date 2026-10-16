int f(int k) {
    static const void *const table[] = { &&a, &&b };
    static const int offsets[] = { &&b - &&a };
    void *next = &&a + offsets[0];
    if (k)
        goto *next;
    goto *table[k];
a:
    return 0;
b:
    return 1;
}
