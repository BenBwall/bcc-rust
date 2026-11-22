int main(void) {
    int values[5] = {3, 8, 1, 9, 4};
    int found = -1;
    for (int i = 0; i < 5; i++) {
        if (values[i] > 7) {
            found = i;
            break;
        }
    }
    return found;
}
