int main(void) {
    int values[5] = {4, 8, 15, 16, 23};
    int i = 0;
    while (i < 5 && values[i] != 15) {
        i++;
    }
    return i;
}
