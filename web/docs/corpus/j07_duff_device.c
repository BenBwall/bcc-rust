int main(void) {
    int from[5] = {1, 2, 3, 4, 5};
    int to[5];
    int *f = from;
    int *t = to;
    int n = (5 + 3) / 4;
    switch (5 % 4) {
    case 0: do { *t++ = *f++;
    case 3:      *t++ = *f++;
    case 2:      *t++ = *f++;
    case 1:      *t++ = *f++;
            } while (--n > 0);
    }
    return to[0] * 10 + to[4];
}
