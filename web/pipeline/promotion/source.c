// Arithmetic never happens in a type narrower than int.
int main(void) {
    char small = 100;
    // both are promoted to int: 200
    int wide = small + small;
    // 200 does not fit in a char
    char narrow = small + small;
    narrow++;
    return wide - narrow;
}
