enum e { A, B };
void stop(void) __attribute__((noreturn));
int some(int x) {
    if (x)
        return 1;
}
int none(void) {
}
int covered(enum e x) {
    switch (x) {
    case A:
        return 1;
    case B:
        return 2;
    }
}
int stops(void) {
    stop();
}
static int unlowered(void) {
}
__attribute__((noreturn)) void returns(int x) {
    if (x)
        stop();
}
_Noreturn void says(void) {
    return;
}
int main(void) {
}
