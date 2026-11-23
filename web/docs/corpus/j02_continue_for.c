int main(void) {
    int sum = 0;
    for (int i = 1; i <= 6; i++) {
        int square = i * i;
        if (i % 2 == 0) {
            continue;
        }
        sum += square;
    }
    return sum;
}
