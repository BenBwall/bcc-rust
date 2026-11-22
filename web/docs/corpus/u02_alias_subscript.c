int main(void) {
    int i = 0;
    int *a = &i;
    return a[i++];
}
