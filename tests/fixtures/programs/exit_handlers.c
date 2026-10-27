/* How a program ends, chosen by MODE: returning from main (0), exit (1),
   quick_exit (2), _Exit (3), _exit (4), and exit from inside an exit
   handler (5). The handlers print as they run; the stream to PATH holds
   unwritten bytes, which only a normal end writes out. The interpreter must
   end as the program built against glibc does: the same output to one
   pipe or file, the same status and the same file.
   Declarations are written out so that no system header is needed. */
typedef struct _IO_FILE FILE;
extern FILE *stdout, *stderr;
int printf(const char *, ...);
int fprintf(FILE *, const char *, ...);
int fputs(const char *, FILE *);
int fflush(FILE *);
FILE *fopen(const char *, const char *);
int atexit(void (*)(void));
int on_exit(void (*)(int, void *), void *);
int at_quick_exit(void (*)(void));
void exit(int);
void quick_exit(int);
void _Exit(int);
void _exit(int);

static void first(void) { printf("first, registered by a constructor\n"); }
static void late(void) { printf("late, registered while the handlers ran\n"); }
static void second(void) {
  printf("second\n");
  atexit(late);
}
static void with_status(int status, void *argument) {
  printf("on_exit status %d argument %s\n", status, (const char *)argument);
}
static void quick_first(void) {
  printf("quick first\n");
  fflush(stdout);
}
static void quick_second(void) { printf("quick second, never written out\n"); }
static void exits_again(void) {
  printf("exits again\n");
  exit(7);
}
static void from_destructor(void) { printf("registered by a destructor\n"); }

__attribute__((destructor)) static void destructor(void) {
  printf("destructor\n");
  atexit(from_destructor);
}
__attribute__((constructor)) static void constructor(void) { atexit(first); }

int main(void) {
  FILE *file = fopen(PATH, "w");
  fputs("held in the buffer", file);
  printf("main\n");
  fprintf(stderr, "stderr at once\n");
  if (atexit(second) != 0 || on_exit(with_status, "text") != 0)
    return 1;
  at_quick_exit(quick_first);
  at_quick_exit(quick_second);
  if (MODE == 5)
    atexit(exits_again);
  switch (MODE) {
  case 1:
    exit(4);
  case 2:
    quick_exit(5);
  case 3:
    _Exit(6);
  case 4:
    _exit(8);
  case 5:
    exit(2);
  }
  return 3;
}
