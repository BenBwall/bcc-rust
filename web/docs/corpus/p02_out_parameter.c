int pick(int **out, int *choice) {
    *out = choice;
    return 0;
}

int main(void) {
    int a = 3;
    int *chosen = 0;
    pick(&chosen, &a);
    return *chosen;
}
