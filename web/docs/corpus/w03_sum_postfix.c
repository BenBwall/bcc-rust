int main(void) {
    int v[3] = {4, 5, 6};
    int *p = v;
    int total = 0;
    while (p < v + 3)
        total += *p++;
    return total;
}
