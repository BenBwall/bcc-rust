#include <stdio.h>

typedef int (*operation)(int, int);

int add(int a, int b) {
    return a + b;
}

int sub(int a, int b) {
    return a - b;
}

struct command {
    char name;
    operation run;
};

operation find(char name) {
    if (name == '+') {
        return add;
    }
    return sub;
}

int main(void) {
    struct command c = {'-', sub};
    printf("%d\n", c.run(9, 4));
    return find('+')(2, 3);
}
