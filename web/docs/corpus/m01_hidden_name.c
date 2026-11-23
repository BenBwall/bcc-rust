int main(void) {
    int x = 1;
    int seen = 0;
    {
        seen = x;
        int x = 2;
        seen = seen * 10 + x;
    }
    return seen * 10 + x;
}
