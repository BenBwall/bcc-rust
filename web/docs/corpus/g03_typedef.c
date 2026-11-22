typedef int (*binop)(int, int);

int add(int a, int b) {
    return a + b;
}

int mul(int a, int b) {
    return a * b;
}

int main(void) {
    binop op = add;
    int sum = op(2, 3);
    op = mul;
    return sum + op(4, 5);
}
