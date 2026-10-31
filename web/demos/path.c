// One small program, followed from its text to a running program.
int square(int n) {
    return n * n;
}

int main(void) {
    int side = 3;
    int area = square(side + 1);
    return area;
}
