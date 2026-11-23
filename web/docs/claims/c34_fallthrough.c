/* A case that falls through into the next. */
int f(int s) {
    int n = 0;
    switch (s) {
    case 0:
        n = 1;
    case 1:
        n += 2;
        break;
    }
    return n;
}
int main(void) { return f(0); }
