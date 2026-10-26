static long factorial(long n) {
    if (n <= 1) return 1;
    return n * factorial(n - 1);
}

static unsigned gcd(unsigned a, unsigned b) {
    while (b) {
        unsigned t = a % b;
        a = b;
        b = t;
    }
    return a;
}

int main(void) { return factorial(5) + gcd(1071, 462); }
