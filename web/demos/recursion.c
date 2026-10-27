// Every call gets its own frame, with its own copy of n.
int factorial(int n) {
    if (n <= 1) {
        return 1;
    }
    int smaller = factorial(n - 1);
    return n * smaller;
}

int main(void) {
    int result = factorial(4);
    return result;
}
