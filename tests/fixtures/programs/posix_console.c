/* The console's descriptors when stdout and stderr are pipes, as the
   tests run programs: what fstat, lseek and isatty report for 1 and 2,
   and write(1) overtaking what stdout buffers. */

#define _GNU_SOURCE 1
#include <errno.h>
#include <stdio.h>
#include <sys/stat.h>
#include <unistd.h>

int main(void) {
    printf("buffered ");
    write(1, "raw ", 4);
    fflush(stdout);
    for (int fd = 1; fd <= 2; fd++) {
        struct stat st;
        errno = 0;
        int r = fstat(fd, &st);
        printf("fstat %d: %d fifo %d mode %o blksize %ld links %ld size %ld\n", fd, r,
               S_ISFIFO(st.st_mode), st.st_mode & 0777, (long)st.st_blksize, (long)st.st_nlink,
               (long)st.st_size);
        errno = 0;
        printf("lseek %d: %ld %d\n", fd, (long)lseek(fd, 0, SEEK_CUR), errno);
        errno = 0;
        printf("isatty %d: %d %d\n", fd, isatty(fd), errno);
    }
    int copy = dup(2);
    struct stat st;
    printf("dup of 2: %d fstat %d fifo %d\n", copy > 2, fstat(copy, &st), S_ISFIFO(st.st_mode));
    write(copy, "to stderr\n", 10);
    fprintf(stderr, "stream\n");
    printf("end\n");
    return 0;
}
