// Arithmetic never happens in a type narrower than int.
int main(void) {
    char small = 100;
    int wide = small + small;     // both chars are promoted to int: 200
    char narrow = small + small;  // 200 does not fit in a char
    narrow++;
    return wide - narrow;
}
