int *larger(int *a, int *b) {
    if (*a > *b) {
        return a;
    }
    return b;
}

int main(void) {
    int x = 3;
    int y = 8;
    *larger(&x, &y) = 0;
    return x * 10 + y;
}
