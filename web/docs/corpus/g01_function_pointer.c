int add(int a, int b) {
    return a + b;
}

int main(void) {
    int (*operation)(int, int) = add;
    return operation(2, 3);
}
