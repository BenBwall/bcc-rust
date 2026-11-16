// Padding makes memcmp of equal structures unreliable.
#include <stdio.h>
#include <string.h>
struct entry { char tag; int count; };
int main(void) {
    struct entry a, b;
    memset(&a, 0x00, sizeof a);
    memset(&b, 0xff, sizeof b);
    a.tag = 7; a.count = 5;
    b.tag = 7; b.count = 5;
    printf("members equal: %d, memcmp equal: %d\n", a.tag == b.tag && a.count == b.count, memcmp(&a, &b, sizeof a) == 0);
    return 0;
}
