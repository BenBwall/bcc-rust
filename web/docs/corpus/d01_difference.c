int main(void) {
    int v[5] = {1, 2, 3, 4, 5};
    int *p = &v[1];
    int *q = &v[4];
    return (int)(q - p) * 10 + (int)(p - q + 5);
}
