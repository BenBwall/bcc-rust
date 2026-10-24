static void cl(int *p);
void g(void) { int x __attribute__((cleanup(cl))) = 0; (void)x; }
