int *make(void) {
    int local = 42;
    return &local;
}

int main(void) {
    int *p = make();
    return *p;
}
