int bump(int *value) {
    int before = *value;
    *value = before + 1;
    return *value;
}

int main(void) {
    int total = 0;
    int values[3] = {2, 4, 6};
    int *cursor = &values[0];

    for (int i = 0; i < 3; i = i + 1) {
        int next = bump(cursor);
        total = total + next;
        cursor = cursor + 1;
    }

    return total;
}
