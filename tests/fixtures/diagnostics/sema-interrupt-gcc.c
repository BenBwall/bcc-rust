/* GCC's checks of a handler, each with its own message. */
__attribute__((interrupt)) void none(void);
__attribute__((interrupt)) void not_pointer(int x);
__attribute__((interrupt)) void not_word(void *p, int e);
__attribute__((interrupt)) void three(void *p, unsigned long e, unsigned long z);
__attribute__((interrupt)) int returns(void *p);
__attribute__((interrupt, naked)) void both(void *p) {}
__attribute__((interrupt)) void handler(void *p);
void caller(void) { handler(0); }
int following;
