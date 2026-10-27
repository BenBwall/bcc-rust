// Walk an array with two pointers: [first, last) is a half-open range.
int sum(int *first, int *last) {
    int total = 0;
    while (first != last) {
        total = total + *first;
        first = first + 1;
    }
    return total;
}

int main(void) {
    int primes[4] = {2, 3, 5, 7};
    int *begin = primes;      // the array decays to &primes[0]
    int *end = primes + 4;    // one past the end: valid to form, not to read
    return sum(begin, end);
}
