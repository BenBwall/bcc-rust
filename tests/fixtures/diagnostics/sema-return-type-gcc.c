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

/* GCC's front end counts a call to a function that does not return even
   where it is statically dead, as PostgreSQL's ereport() leaves one behind
   `if (__builtin_constant_p(15) && 15 >= 21)`. */
void halts(void) __attribute__((noreturn));
int poll_once(void);
static int dead_unreachable(void) { for (;;) { if (0) __builtin_unreachable(); } }
static int dead_halt(void) { for (;;) { if (0) halts(); } }
static int folded_unreachable(void) { for (;;) { if (__builtin_constant_p(15) && 15 >= 21) __builtin_unreachable(); } }
static int after_break(int x) { while (x) { break; halts(); } }
static int only_polls(void) { for (;;) { poll_once(); } }
int use_statics(void) { return dead_unreachable() + dead_halt() + folded_unreachable() + after_break(1) + only_polls(); }
