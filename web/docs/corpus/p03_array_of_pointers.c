int main(void) {
    int a = 1;
    int b = 2;
    int c = 3;
    int *slots[3] = {&a, &b, &c};
    *slots[1] = 20;
    return b;
}
