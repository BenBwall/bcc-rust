/* Clang's checks of a handler, which it stops at the first of, and its
   warning of the registers a call makes a handler save. */
__attribute__((interrupt)) void none(void);
__attribute__((interrupt)) void not_pointer(int x);
__attribute__((interrupt)) void not_word(void *p, long e);
__attribute__((interrupt)) void three(void *p, unsigned long e, unsigned long z);
__attribute__((interrupt)) int returns(void *p);
__attribute__((interrupt)) void handler(void *p);
void caller(void) { handler(0); }
void plain(void);
__attribute__((interrupt)) void calling(void *p) { plain(); }
__attribute__((no_caller_saved_registers)) void saving(void) { plain(); }
int following;
