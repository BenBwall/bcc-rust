int main(void) {
    int n = 0;
    int digits = 0;
    do {
        digits++;
        n = n / 10;
    } while (n != 0);
    return digits;
}
