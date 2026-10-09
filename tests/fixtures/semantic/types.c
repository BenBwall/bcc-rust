typedef unsigned long Size;
struct Node {int value; struct Node *next;};
enum Mode {Idle, Busy=4, Done};
static const struct Node *head;
extern const struct Node *head;
int (*callbacks[3])(Size, const char *, ...);
int f(int values[static const 4], int (*cb)(int)) {
    int local;
    { const Size count = 2; int values[count]; }
    for (int i=0; i<3; ++i) { struct Node *node; }
    return local;
}
struct Bits {char tag; unsigned flags:5; unsigned :0; union {int i; float f;};};
