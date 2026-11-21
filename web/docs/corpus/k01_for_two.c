int main(void) {
    int values[4] = {1, 2, 3, 4};
    int i;
    int j;
    for (i = 0, j = 3; i < j; i++, j--) {
        int saved = values[i];
        values[i] = values[j];
        values[j] = saved;
    }
    return values[0];
}
