/* GCC saves no SSE state, so it wants -mgeneral-regs-only of these. */
__attribute__((interrupt)) void interrupt(void *p) {}
__attribute__((interrupt)) void exception(void *p, unsigned long e) {}
__attribute__((no_caller_saved_registers)) void saving(void) {}
int following;
