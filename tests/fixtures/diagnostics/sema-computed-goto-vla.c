void f(int n) {
    void *p = &&inside;
    goto *p;
    {
        int vla[n];
    inside:
        vla[0] = 1;
    }
}
