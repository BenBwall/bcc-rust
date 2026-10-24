int main(void) {
    unsigned char c = 255;
    c++;
    unsigned u = 0;
    u--;
    int x = 0;
    0 && (x = 1);
    1 || (x = 2);
    int chosen = x == 0 ? 7 : 9;
    return (c == 0) * 10 + (u == 4294967295u) + chosen;
}
