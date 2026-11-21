// `-Werror` keeps naming `-Werror=pedantic` after a pragma naming another
// option, and after a `push` and `pop`, even around one naming `-Wpedantic`.
void g(void);
#pragma GCC diagnostic ignored "-Wformat"
void *p = (void *)g;
#pragma GCC diagnostic push
#pragma GCC diagnostic warning "-Wlong-long"
void *q = (void *)g;
#pragma GCC diagnostic pop
#pragma GCC diagnostic push
#pragma GCC diagnostic error "-Wpedantic"
#pragma GCC diagnostic pop
void *r = (void *)g;
// A pragma naming `-Wpedantic` keeps its report a warning.
#pragma GCC diagnostic warning "-Wpedantic"
void *s = (void *)g;
