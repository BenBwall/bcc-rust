int n = 3;
struct file_scope { int a[n]; };
int f(int m) {
    struct inner { int a; int b[m]; int c; } s;
    union { int (*p)[m]; char c; } u;
    s.c = 1;
    u.c = 2;
    return s.c + u.c;
}
