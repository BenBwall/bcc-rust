/* GCC names the function a diagnostic is in when it changes, and says
   `At top level` on leaving one. */
static int unused_parameter(int a) { return 0; }
void main(void) { }
int first(void) {
  int *p = 1;
  return *p;
}
int second(void) {
  int *q = 2;
  return *q;
}
int *file_scope = 3;
int third(void) { int *r = 4; return *r; }
