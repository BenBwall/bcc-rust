int apply(int (*f)(int), int x) {
    return f(x);
}

int square(int x) {
    return x * x;
}

int negate(int x) {
    return -x;
}

int main(void) {
    return apply(square, 7) + apply(negate, 9);
}
