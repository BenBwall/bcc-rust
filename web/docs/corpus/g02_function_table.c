int add(int a, int b) {
    return a + b;
}

int sub(int a, int b) {
    return a - b;
}

int main(void) {
    int (*table[2])(int, int) = {add, sub};
    return table[1](7, 2);
}
