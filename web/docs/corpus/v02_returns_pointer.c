int increment(int x) {
    return x + 1;
}

int decrement(int x) {
    return x - 1;
}

int (*pick(int up))(int) {
    if (up) {
        return increment;
    }
    return decrement;
}

int main(void) {
    int (*step)(int) = pick(1);
    return step(10) + pick(0)(10);
}
