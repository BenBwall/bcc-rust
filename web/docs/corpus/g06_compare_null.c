int add(int a, int b) {
    return a + b;
}

int main(void) {
    int (*f)(int, int) = 0;
    int r = 0;
    if (f == 0)
        r = 1;
    f = add;
    if (f == add)
        r = r + f(3, 4);
    return r;
}
