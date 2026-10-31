// One small program, from its text to machine code.
int square(int n) {
    return n * n;
}

int main(void) {
    int side = 3;
    int area = square(side + 1);
    return area;
}
