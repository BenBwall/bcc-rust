#include <stdio.h>

int main(void) {
    char *names[3] = {"Ada", "Grace", "Linus"};
    for (int i = 0; i < 3; i++) {
        printf("%s\n", names[i]);
    }
    return 0;
}
