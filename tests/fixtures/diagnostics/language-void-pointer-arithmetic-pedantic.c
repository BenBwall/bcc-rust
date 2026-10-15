int arr[8];
static long d = (void *)&arr[3] - (void *)&arr[0];
int f(void *p) {
    void *q = &p[2];
    q += 1;
    return (int)sizeof(p[1]) + (int)(q - p) + (int)d;
}
