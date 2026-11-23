int main(void) {
    int i = 0;
    int skipped = 0;
    do {
        i++;
        if (i == 3) {
            skipped = i;
            continue;
        }
    } while (i < 5);
    return i * 10 + skipped;
}
