/* A variable used more often than a sort fits on the stack: Clang reports
   only the first use, and ordering them allocates only in arenas. */
void use(int);
void many(int a) {
  int x;
  if (a) x = 1;
  use(x); use(x); use(x); use(x); use(x); use(x); use(x); use(x); use(x); use(x); use(x); use(x);
  use(x); use(x); use(x); use(x); use(x); use(x); use(x); use(x); use(x); use(x); use(x); use(x);
  use(x); use(x); use(x); use(x); use(x); use(x); use(x); use(x); use(x); use(x); use(x); use(x);
  use(x); use(x); use(x); use(x); use(x); use(x); use(x); use(x); use(x); use(x); use(x); use(x);
  use(x); use(x); use(x); use(x); use(x); use(x); use(x); use(x); use(x); use(x); use(x); use(x);
  use(x); use(x); use(x); use(x); use(x); use(x); use(x); use(x); use(x); use(x); use(x); use(x);
  use(x); use(x); use(x); use(x); use(x); use(x); use(x); use(x); use(x); use(x); use(x); use(x);
  use(x); use(x); use(x); use(x); use(x); use(x); use(x); use(x); use(x); use(x); use(x); use(x);
  use(x); use(x); use(x); use(x); use(x); use(x); use(x); use(x); use(x); use(x); use(x); use(x);
  use(x); use(x); use(x); use(x); use(x); use(x); use(x); use(x); use(x); use(x); use(x); use(x);
  use(x); use(x); use(x); use(x); use(x); use(x); use(x); use(x); use(x); use(x); use(x); use(x);
  use(x); use(x); use(x); use(x); use(x); use(x); use(x); use(x); use(x); use(x); use(x); use(x);
  use(x); use(x); use(x); use(x); use(x); use(x); use(x); use(x); use(x); use(x); use(x); use(x);
  use(x); use(x); use(x); use(x); use(x); use(x); use(x); use(x); use(x); use(x); use(x); use(x);
  use(x); use(x); use(x); use(x); use(x); use(x); use(x); use(x); use(x); use(x); use(x); use(x);
  use(x); use(x); use(x); use(x); use(x); use(x); use(x); use(x); use(x); use(x); use(x); use(x);
  use(x); use(x); use(x); use(x); use(x); use(x); use(x); use(x); use(x); use(x); use(x); use(x);
  use(x); use(x); use(x); use(x); use(x); use(x); use(x); use(x); use(x); use(x); use(x); use(x);
}
