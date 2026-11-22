int main(void) {
    int a[4] = {1, 2, 3, 4};
    int *lo = a;
    int *hi = a + 3;
    while (lo < hi) {
        int t = *lo;
        *lo++ = *hi;
        *hi-- = t;
    }
    return a[0] * 1000 + a[1] * 100 + a[2] * 10 + a[3];
}
