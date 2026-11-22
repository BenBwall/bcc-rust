enum { SIZE = 4 };

int main(void) {
    int squares[SIZE];
    int total = 0;
    for (int i = 0; i < SIZE; i++) {
        squares[i] = i * i;
        total += squares[i];
    }
    return total;
}
