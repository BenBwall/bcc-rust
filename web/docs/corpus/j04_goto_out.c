int main(void) {
    int found = -1;
    for (int row = 0; row < 3; row++) {
        for (int col = 0; col < 3; col++) {
            int cell = row * 3 + col;
            if (cell == 5) {
                found = cell;
                goto done;
            }
        }
    }
done:
    return found;
}
