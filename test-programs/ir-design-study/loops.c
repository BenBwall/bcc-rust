static int sum(int n) {
    int total = 0;
    for (int i = 0; i <= n; i++) {
        if (i == 2) continue;
        {
            int term = i;
            if (term > 8) break;
            total += term;
        }
    }
    return total;
}

int main(void) { return sum(10); }
