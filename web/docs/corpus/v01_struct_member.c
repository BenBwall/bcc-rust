int twice(int x) {
    return 2 * x;
}

int square(int x) {
    return x * x;
}

struct ops {
    int (*apply)(int);
    int count;
};

int main(void) {
    struct ops s = {twice, 1};
    struct ops *p = &s;
    int a = s.apply(3);
    p->apply = square;
    return a + p->apply(3);
}
