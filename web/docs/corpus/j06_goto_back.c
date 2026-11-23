int main(void) {
    int tries = 0;
again:
    tries++;
    int square = tries * tries;
    if (square < 10) {
        goto again;
    }
    return tries;
}
