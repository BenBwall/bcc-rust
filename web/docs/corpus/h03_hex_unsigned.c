int main(void) {
    unsigned int all = 0xFFFFFFFF;
    long big = 0x7FFFFFFFFFFFFFFF;
    return all == 4294967295u && big > 0;
}
