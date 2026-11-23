int scores[3] = {40, 75, 60};

int *best(void) {
    int *top = &scores[0];
    for (int i = 1; i < 3; i++) {
        if (scores[i] > *top) {
            top = &scores[i];
        }
    }
    return top;
}

int main(void) {
    int *b = best();
    *b = *b + 5;
    return scores[1];
}
