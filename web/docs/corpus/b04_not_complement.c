int main(void) {
    unsigned char byte = 15;
    int inverted = ~byte;
    return inverted & 255;
}
