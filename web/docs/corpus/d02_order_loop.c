int main(void) {
    int v[3] = {1, 2, 3};
    int *p;
    int n = 0;
    for (p = v; p < v + 3; p++)
        n += *p;
    return n * 10 + (p == v + 3) + (v + 1 <= p);
}
