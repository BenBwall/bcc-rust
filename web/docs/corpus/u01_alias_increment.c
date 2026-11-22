int main(void) {
    int i = 0;
    int *p = &i;
    *p = i++;
    return i;
}
