// Signed int arithmetic has to stay within INT_MAX.
int main(void) {
    int big = 2147483646;
    for (int i = 0; i < 3; i++) {
        big = big + 1;
    }
    return big;
}
