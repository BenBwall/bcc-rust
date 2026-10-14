int main(void) {
    unsigned a = 16;
    unsigned r = a % 4u;
    if (r == 1)
        return 1;
    return 0;
}
