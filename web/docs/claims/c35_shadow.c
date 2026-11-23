int main(void) {
    int x = 1;
    {
        int x = 2;
        (void)x;
    }
    return x;
}
