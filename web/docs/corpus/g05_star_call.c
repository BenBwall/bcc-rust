int twice(int x) {
    return 2 * x;
}

int main(void) {
    int (*f)(int) = &twice;
    return (*f)(20) + f(1);
}
