typedef int (*unary)(int);

int add(int a, int b) {
    return a + b;
}

int main(void) {
    unary f = (unary)add;
    return f(3);
}
