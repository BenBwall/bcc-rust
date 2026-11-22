int main(void) {
    int v[3];
    int *p = v;
    *p++ = 1;
    *p++ = 2;
    *p = 3;
    return v[0] * 100 + v[1] * 10 + v[2];
}
