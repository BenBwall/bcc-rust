// An unsigned int as a row of on/off flags, one bit each.
#include <stdio.h>

#define READ 0x4     // binary 100
#define WRITE 0x2    // binary 010
#define RUN 0x1      // binary 001

int main(void) {
    unsigned int mode = 0;
    mode |= READ;            // | sets a bit
    mode |= WRITE | RUN;     // two at once: 111
    mode &= ~WRITE;          // & with ~ clears one: 101
    mode ^= RUN;             // ^ flips one: 100
    int can_read = (mode & READ) != 0;   // & tests one
    printf("mode %x: read %d, write %d\n", mode, can_read, (mode & WRITE) != 0);
    return 0;
}
