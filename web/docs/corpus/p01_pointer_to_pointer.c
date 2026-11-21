int main(void) {
    int x = 1;
    int *p = &x;
    int **pp = &p;
    **pp = 5;
    return x;
}
