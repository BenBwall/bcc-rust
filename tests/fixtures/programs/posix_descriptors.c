/* POSIX descriptors and their streams, one case per MODE, with stdout and
   stderr sharing one file: write(1) and write(2) interleaving with the
   buffered standard streams, dup2 and close under stdout, freopen, fdopen
   and fileno, files through descriptors with every open flag the
   interpreter passes on, pipes, and errno on each failure. PATH names a
   scratch file. A test harness may leave descriptors above 2 open, so
   only the numbers 0 to 2 and how the others relate are printed. */

#define _GNU_SOURCE 1
#include <errno.h>
#include <fcntl.h>
#include <stdio.h>
#include <string.h>
#include <sys/stat.h>
#include <unistd.h>

/* What PATH holds, on stderr. */
static void show(void) {
    char buf[256] = {0};
    int fd = open(PATH, O_RDONLY);
    ssize_t n = read(fd, buf, sizeof buf - 1);
    close(fd);
    fprintf(stderr, "[%zd: %s]\n", n, buf);
}

int main(void) {
#if MODE == 0
    /* Raw writes reach the file before what the streams buffer. */
    printf("A");
    write(1, "B", 1);
    fprintf(stderr, "C");
    printf("D\n");
    write(2, "E", 1);
    fflush(stdout);
    write(1, "F\n", 2);
    printf("fileno %d %d %d\n", fileno(stdin), fileno(stdout), fileno(stderr));
    int d = dup(1);
    FILE *f = fdopen(d, "w");
    printf("dup %d fileno %d\n", d > 2, fileno(f) == d);
    fprintf(f, "via fdopen\n");
    printf("after\n");
    fflush(f);
    errno = 0;
    printf("isatty %d %d\n", isatty(1), errno);
    fclose(f);
    errno = 0;
    printf("closed dup %d %d\n", close(d), errno);
#elif MODE == 1
    /* dup2 a file onto stdout and back: the buffer follows descriptor 1. */
    printf("before ");
    int saved = dup(1);
    int fd = open(PATH, O_WRONLY | O_CREAT | O_TRUNC, 0600);
    dup2(fd, 1);
    close(fd);
    printf("into file\n");
    write(1, "raw\n", 4);
    fflush(stdout);
    dup2(saved, 1);
    close(saved);
    printf("back\n");
    show();
#elif MODE == 2
    /* close(1) loses what stdout writes out; a new open reuses 1. */
    printf("lost");
    close(1);
    int n = printf("x");
    errno = 0;
    int r = fflush(stdout);
    fprintf(stderr, "n %d fflush %d errno %d ferror %d\n", n, r, errno, ferror(stdout));
    clearerr(stdout);
    int fd = open(PATH, O_WRONLY | O_CREAT | O_TRUNC, 0600);
    fprintf(stderr, "reused %d\n", fd);
    printf("to file via stdout\n");
    fflush(stdout);
    show();
#elif MODE == 3
    /* freopen keeps descriptor 1, so write(1) follows it. */
    printf("a\n");
    freopen(PATH, "w", stdout);
    fprintf(stderr, "fileno %d\n", fileno(stdout));
    printf("in file\n");
    write(1, "raw\n", 4);
    fflush(stdout);
    show();
#elif MODE == 4
    /* A buffered stderr and write(2), then stderr on descriptor 1. */
    setvbuf(stderr, NULL, _IOFBF, 64);
    fprintf(stderr, "buffered ");
    write(2, "raw2 ", 5);
    printf("out ");
    write(1, "raw1 ", 5);
    fflush(stderr);
    dup2(1, 2);
    fprintf(stderr, "e-on-1 ");
    fflush(stderr);
    printf("end\n");
#elif MODE == 5
    /* Streams on both ends of a pipe. */
    int p[2];
    printf("pipe %d %d %d\n", pipe(p), p[0] > 2, p[1] > p[0]);
    printf("write %zd\n", write(p[1], "hello", 5));
    char raw[16] = {0};
    printf("read %zd %s\n", read(p[0], raw, sizeof raw), raw);
    FILE *w = fdopen(p[1], "w");
    FILE *r = fdopen(p[0], "r");
    fprintf(w, "line one\nline two\n");
    fclose(w);
    char buf[64];
    while (fgets(buf, sizeof buf, r))
        printf("got %s", buf);
    printf("eof %d\n", feof(r));
    fclose(r);
    errno = 0;
    printf("closed %d %d\n", close(p[0]), errno);
    errno = 0;
    char c;
    printf("read wronly %zd %d\n", read(1, &c, 1), errno);
#elif MODE == 6
    /* A line-buffered stream on a duplicate of stdout. */
    FILE *o = fdopen(dup(1), "w");
    setvbuf(o, NULL, _IOLBF, 0);
    printf("std1 ");
    fprintf(o, "dup1\n");
    printf("std2\n");
    fprintf(o, "dup2 ");
    fflush(stdout);
    fclose(o);
    printf("done\n");
#elif MODE == 7
    /* Closing the descriptor under a stream fails its fclose. */
    FILE *f = fopen(PATH, "w");
    fputs("pending", f);
    printf("close %d\n", close(fileno(f)));
    errno = 0;
    int r = fclose(f);
    printf("fclose %d errno %d\n", r, errno);
#elif MODE == 8
    /* Files through descriptors. */
    unlink(PATH);
    errno = 0;
    int fd = open(PATH, O_WRONLY | O_CREAT | O_EXCL, 0640);
    printf("open %d %d\n", fd > 2, errno);
    errno = 0;
    printf("excl %d %d\n", open(PATH, O_WRONLY | O_CREAT | O_EXCL, 0640), errno);
    printf("write %zd\n", write(fd, "0123456789", 10));
    printf("lseek %ld\n", (long)lseek(fd, 3, SEEK_SET));
    printf("write %zd\n", write(fd, "ab", 2));
    printf("lseek end %ld\n", (long)lseek(fd, 0, SEEK_END));
    errno = 0;
    printf("lseek negative %ld %d\n", (long)lseek(fd, -100, SEEK_SET), errno);
    errno = 0;
    printf("lseek whence %ld %d\n", (long)lseek(fd, 0, 7), errno);
    struct stat st;
    printf("fstat %d %o %ld %ld\n", fstat(fd, &st), st.st_mode & 07777, (long)st.st_size,
           (long)st.st_nlink);
    printf("stat %d %d %ld\n", stat(PATH, &st), S_ISREG(st.st_mode), (long)st.st_size);
    errno = 0;
    printf("stat missing %d %d\n", stat(PATH "-missing/x", &st), errno);
    printf("lstat %d %d\n", lstat(PATH, &st), S_ISREG(st.st_mode));
    errno = 0;
    printf("read wronly %zd %d\n", read(fd, &st, 1), errno);
    FILE *f = fdopen(fd, "a");
    printf("fdopen %d fileno %d\n", f != 0, fileno(f) == fd);
    fputs("tail\n", f);
    write(fd, "RAW\n", 4);
    fclose(f);
    errno = 0;
    printf("closed %d %d\n", close(fd), errno);
    int r = open(PATH, O_RDONLY);
    char buf[64] = {0};
    printf("read %zd [%s]\n", read(r, buf, sizeof buf - 1), buf);
    errno = 0;
    printf("write rdonly %zd %d\n", write(r, "x", 1), errno);
    errno = 0;
    printf("fdopen w on rdonly %d %d\n", fdopen(r, "w") != 0, errno);
    errno = 0;
    printf("fdopen bad mode %d %d\n", fdopen(r, "q") != 0, errno);
    errno = 0;
    printf("fdopen closed %d %d\n", fdopen(99, "r") != 0, errno);
    int s = dup(r);
    printf("dup %d shared %ld %ld\n", s > r, (long)lseek(r, 2, SEEK_SET),
           (long)lseek(s, 0, SEEK_CUR));
    printf("dup2 %d\n", dup2(r, 60));
    printf("dup2 same %d\n", dup2(60, 60));
    errno = 0;
    printf("dup2 bad %d %d\n", dup2(50, 61), errno);
    errno = 0;
    printf("dup3 same %d %d\n", dup3(60, 60, 0), errno);
    printf("dup3 %d\n", dup3(60, 62, O_CLOEXEC));
    FILE *h = fopen(PATH, "r");
    printf("fopen fileno %d\n", fileno(h) > s);
    fclose(h);
    int t = open(PATH, O_RDWR | O_TRUNC | O_APPEND);
    write(t, "x", 1);
    lseek(t, 0, SEEK_SET);
    write(t, "y", 1);
    printf("truncate and append %ld\n", (long)lseek(t, 0, SEEK_CUR));
    errno = 0;
    printf("isatty file %d %d\n", isatty(t), errno);
    errno = 0;
    printf("isatty closed %d %d\n", isatty(77), errno);
    errno = 0;
    printf("close closed %d %d\n", close(77), errno);
    errno = 0;
    printf("read closed %zd %d\n", read(77, buf, 1), errno);
    errno = 0;
    printf("write closed %zd %d\n", write(77, buf, 1), errno);
    errno = 0;
    printf("lseek closed %ld %d\n", (long)lseek(77, 0, SEEK_SET), errno);
    errno = 0;
    printf("fstat closed %d %d\n", fstat(77, &st), errno);
    printf("unlink %d\n", unlink(PATH));
    errno = 0;
    printf("unlink again %d %d\n", unlink(PATH), errno);
    errno = 0;
    printf("openat missing %d %d\n", openat(AT_FDCWD, PATH "-missing/y", O_RDONLY), errno);
    errno = 0;
    printf("openat on stdout %d %d\n", openat(1, "relative", O_RDONLY), errno);
    errno = 0;
    printf("openat closed %d %d\n", openat(88, "relative", O_RDONLY), errno);
    int c = creat(PATH, 0600);
    printf("creat %d %zd\n", c > 2, write(c, "c", 1));
    close(c);
    unlink(PATH);
#endif
    return 0;
}
