int main(void) {
    unsigned int flags = 0;
    flags |= 4;
    flags |= 1;
    flags &= 6;
    flags ^= 3;
    return flags;
}
