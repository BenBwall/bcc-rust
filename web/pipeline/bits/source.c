// One bit for each on/off flag.
#include <stdio.h>

#define READ 0x4    // bit 2: 100
#define WRITE 0x2   // bit 1: 010
#define RUN 0x1     // bit 0: 001

int main(void) {
    unsigned int mode = 0;
    // | sets, & ~ clears, ^ flips,
    // and & tests a bit.
    mode |= READ;
    mode |= WRITE | RUN;
    mode &= ~WRITE;
    mode ^= RUN;
    int can_read = (mode & READ) != 0;
    printf("mode %x: read %d, write %d\n",
           mode, can_read, (mode & WRITE) != 0);
    return 0;
}
