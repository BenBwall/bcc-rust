/* POSIX descriptors and their streams, one case per MODE, with stdout and
   stderr sharing one file: write(1) and write(2) interleaving with the
   buffered standard streams, dup2 and close under stdout, freopen, fdopen
   and fileno, files through descriptors with every open flag the
   interpreter passes on, pipes, and errno on each failure. PATH names a
   scratch file. A test harness may leave descriptors above 2 open, so
   only the numbers 0 to 2 and how the others relate are printed. Every
   call is sequenced before errno or its results are read. */

#define _GNU_SOURCE 1
#include <errno.h>
#include <fcntl.h>
#include <stdio.h>
#include <string.h>
#include <sys/stat.h>
#include <unistd.h>

/* Prints CALL's value and the errno it leaves, clearing errno first. */
#define SHOW(label, call)                                                      \
    do {                                                                       \
        errno = 0;                                                             \
        long value_ = (long)(call);                                            \
        int errno_ = errno;                                                    \
        printf("%s %ld %d\n", label, value_, errno_);                          \
    } while (0)

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
    int in = fileno(stdin);
    int out = fileno(stdout);
    int err = fileno(stderr);
    printf("fileno %d %d %d\n", in, out, err);
    int d = dup(1);
    FILE *f = fdopen(d, "w");
    int number = fileno(f);
    printf("dup %d fileno %d\n", d > 2, number == d);
    fprintf(f, "via fdopen\n");
    printf("after\n");
    fflush(f);
    SHOW("isatty", isatty(1));
    fclose(f);
    SHOW("closed dup", close(d));
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
    int e = errno;
    int failed = ferror(stdout);
    fprintf(stderr, "n %d fflush %d errno %d ferror %d\n", n, r, e, failed);
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
    int number = fileno(stdout);
    fprintf(stderr, "fileno %d\n", number);
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
    int p[2] = {-1, -1};
    int made = pipe(p);
    printf("pipe %d %d %d\n", made, p[0] > 2, p[1] > p[0]);
    SHOW("write", write(p[1], "hello", 5));
    char raw[16] = {0};
    long got = read(p[0], raw, sizeof raw - 1);
    printf("read %ld %s\n", got, raw);
    FILE *w = fdopen(p[1], "w");
    FILE *r = fdopen(p[0], "r");
    fprintf(w, "line one\nline two\n");
    fclose(w);
    char buf[64];
    while (fgets(buf, sizeof buf, r))
        printf("got %s", buf);
    int end = feof(r);
    printf("eof %d\n", end);
    fclose(r);
    SHOW("closed", close(p[0]));
    char c = 0;
    SHOW("read wronly", read(1, &c, 1));
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
    SHOW("close", close(fileno(f)));
    SHOW("fclose", fclose(f));
#elif MODE == 8
    /* Files through descriptors. */
    unlink(PATH);
    errno = 0;
    int fd = open(PATH, O_WRONLY | O_CREAT | O_EXCL, 0640);
    int e = errno;
    printf("open %d %d\n", fd > 2, e);
    SHOW("excl", open(PATH, O_WRONLY | O_CREAT | O_EXCL, 0640));
    SHOW("write", write(fd, "0123456789", 10));
    SHOW("lseek", lseek(fd, 3, SEEK_SET));
    SHOW("write", write(fd, "ab", 2));
    SHOW("lseek end", lseek(fd, 0, SEEK_END));
    SHOW("lseek negative", lseek(fd, -100, SEEK_SET));
    SHOW("lseek whence", lseek(fd, 0, 7));
    struct stat st;
    memset(&st, 0, sizeof st);
    SHOW("fstat", fstat(fd, &st));
    printf("  mode %o size %ld links %ld\n", (unsigned)(st.st_mode & 07777), (long)st.st_size,
           (long)st.st_nlink);
    memset(&st, 0, sizeof st);
    SHOW("stat", stat(PATH, &st));
    printf("  regular %d size %ld\n", S_ISREG(st.st_mode), (long)st.st_size);
    SHOW("stat missing", stat(PATH "-missing/x", &st));
    memset(&st, 0, sizeof st);
    SHOW("lstat", lstat(PATH, &st));
    printf("  regular %d\n", S_ISREG(st.st_mode));
    SHOW("read wronly", read(fd, &st, 1));
    FILE *f = fdopen(fd, "a");
    int number = fileno(f);
    printf("fdopen %d fileno %d\n", f != 0, number == fd);
    fputs("tail\n", f);
    write(fd, "RAW\n", 4);
    fclose(f);
    SHOW("closed", close(fd));
    int r = open(PATH, O_RDONLY);
    char buf[64] = {0};
    long got = read(r, buf, sizeof buf - 1);
    printf("read %ld [%s]\n", got, buf);
    SHOW("write rdonly", write(r, "x", 1));
    SHOW("fdopen w on rdonly", fdopen(r, "w") != 0);
    SHOW("fdopen bad mode", fdopen(r, "q") != 0);
    SHOW("fdopen closed", fdopen(99, "r") != 0);
    int s = dup(r);
    long moved = lseek(r, 2, SEEK_SET);
    long shared = lseek(s, 0, SEEK_CUR);
    printf("dup %d shared %ld %ld\n", s > r, moved, shared);
    SHOW("dup2", dup2(r, 60));
    SHOW("dup2 same", dup2(60, 60));
    SHOW("dup2 bad", dup2(50, 61));
    SHOW("dup3 same", dup3(60, 60, 0));
    SHOW("dup3", dup3(60, 62, O_CLOEXEC));
    FILE *h = fopen(PATH, "r");
    number = fileno(h);
    printf("fopen fileno %d\n", number > s);
    fclose(h);
    int t = open(PATH, O_RDWR | O_TRUNC | O_APPEND);
    write(t, "x", 1);
    lseek(t, 0, SEEK_SET);
    write(t, "y", 1);
    SHOW("truncate and append", lseek(t, 0, SEEK_CUR));
    SHOW("isatty file", isatty(t));
    SHOW("isatty closed", isatty(77));
    SHOW("close closed", close(77));
    SHOW("read closed", read(77, buf, 1));
    SHOW("write closed", write(77, buf, 1));
    SHOW("lseek closed", lseek(77, 0, SEEK_SET));
    SHOW("fstat closed", fstat(77, &st));
    SHOW("unlink", unlink(PATH));
    SHOW("unlink again", unlink(PATH));
    SHOW("openat missing", openat(AT_FDCWD, PATH "-missing/y", O_RDONLY));
    SHOW("openat on stdout", openat(1, "relative", O_RDONLY));
    SHOW("openat closed", openat(88, "relative", O_RDONLY));
    int c = creat(PATH, 0600);
    long wrote = write(c, "c", 1);
    printf("creat %d %ld\n", c > 2, wrote);
    close(c);
    unlink(PATH);
#elif MODE == 9
    /* fdopen of an O_APPEND descriptor in "a" leaves the shared offset;
       one without O_APPEND gets it and starts at the end. */
    int fd = open(PATH, O_RDWR | O_CREAT | O_TRUNC | O_APPEND, 0600);
    write(fd, "abcdef", 6);
    lseek(fd, 1, SEEK_SET);
    FILE *f = fdopen(fd, "a");
    SHOW("fdopen appending", lseek(fd, 0, SEEK_CUR));
    int g = open(PATH, O_RDWR);
    lseek(g, 2, SEEK_SET);
    FILE *h = fdopen(g, "a");
    SHOW("fdopen adding O_APPEND", lseek(g, 0, SEEK_CUR));
    /* With O_APPEND now set, a raw write after a seek still goes last. */
    lseek(g, 0, SEEK_SET);
    write(g, "!", 1);
    fclose(h);
    fclose(f);
    show();
    /* ftell uses the offset the stream cached, whatever lseek did. */
    FILE *s = fopen(PATH, "w+");
    fputs("abcdefghij", s);
    fflush(s);
    fseek(s, 0, SEEK_SET);
    int c = fgetc(s);
    lseek(fileno(s), 2, SEEK_SET);
    SHOW("ftell cached", ftell(s));
    printf("read %c\n", c);
    fclose(s);
    /* fstatat relative to a directory descriptor: PATH from the root. */
    struct stat st;
    memset(&st, 0, sizeof st);
    int root = open("/", O_RDONLY | O_DIRECTORY);
    SHOW("fstatat", fstatat(root, &PATH[1], &st, AT_SYMLINK_NOFOLLOW));
    printf("  regular %d size %ld\n", S_ISREG(st.st_mode), (long)st.st_size);
    memset(&st, 0, sizeof st);
    SHOW("fstatat empty path", fstatat(root, "", &st, AT_EMPTY_PATH));
    printf("  directory %d\n", S_ISDIR(st.st_mode));
    SHOW("fstatat empty without flag", fstatat(root, "", &st, 0));
    SHOW("fstatat bad flag", fstatat(root, &PATH[1], &st, 0x4));
    SHOW("fstatat on stdout", fstatat(1, "relative", &st, 0));
    SHOW("fstatat missing", fstatat(AT_FDCWD, PATH "-missing", &st, 0));
    close(root);
    unlink(PATH);
#endif
    return 0;
}
