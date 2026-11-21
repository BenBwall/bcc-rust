int is_even(int n) {
    return n % 2 == 0;
}

int main(void) {
    int n = 4;
    if (is_even(n) && n > 2) {
        return 1;
    }
    return 0;
}
