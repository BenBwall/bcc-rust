struct point {
    int x;
    int y;
};

struct entry {
    char tag;
    int count;
};

int calls;
struct point origin = {1, 2};

struct point moved(struct point p, int dx) {
    static int total;
    total = total + dx;
    calls = calls + 1;
    p.x = p.x + dx;
    return p;
}

int main(void) {
    struct point a = origin;
    struct point b = moved(a, 3);
    struct point *at = &b;
    at->y = at->y + 10;
    int *px = &at->x;
    *px = *px * 2;
    struct entry e = {.count = 5};
    e.tag = 7;
    b = moved(b, 4);
    return b.x + b.y + calls + e.count;
}
