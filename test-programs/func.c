#include <stdio.h>
#include <stdlib.h>
#define BAR() 1
#define FOO BAR() ## 7


int main() {
    int x = FOO;
}