// Two constraint violations the compilers refuse by default: an int
// initializing a pointer (C99 §6.5.16.1p1 via §6.7.8p11; GCC 14 makes
// -Wint-conversion an error) and ++ on a const object (§6.5.2.4p1). The
// glossary's "constraint" and the Const note.
int main(void) {
    const int x = 1;
    int *p = 1;
    x++;
    return p == 0;
}
