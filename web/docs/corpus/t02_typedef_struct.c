typedef struct {
    int x;
    int y;
} point;

typedef struct node node;
struct node {
    int value;
    node *next;
};

int main(void) {
    point p = {3, 4};
    node second = {20, 0};
    node first = {10, &second};
    return p.x * p.y + first.next->value;
}
