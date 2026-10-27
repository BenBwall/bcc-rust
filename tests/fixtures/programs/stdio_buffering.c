/* setvbuf, setbuf, setbuffer and setlinebuf on the standard streams and a
   file, chosen by MODE, with output interleaved on stdout and stderr: what
   reaches one pipe or file in which order shows when glibc writes each
   buffer out (a full 4096-byte block, a newline, at once, or a custom
   size). Writes to a file through a small buffer show where it is written
   out, and ftell counts the bytes it holds. MODE 12 writes wide characters
   to a stdout given a 64-byte buffer, whose wide put area holds a quarter
   as many, and ends with _Exit, losing what it holds. The interpreter must
   write what the program built against glibc writes.
   Declarations are written out so that no system header is needed. */
typedef struct _IO_FILE FILE;
typedef unsigned long size_t;
extern FILE *stdout, *stderr;
int printf(const char *, ...);
int fprintf(FILE *, const char *, ...);
int putchar(int);
int puts(const char *);
int fputs(const char *, FILE *);
size_t fwrite(const void *, size_t, size_t, FILE *);
size_t fread(void *, size_t, size_t, FILE *);
FILE *fopen(const char *, const char *);
int fclose(FILE *);
long ftell(FILE *);
int setvbuf(FILE *, char *, int, size_t);
void setbuf(FILE *, char *);
void setbuffer(FILE *, char *, size_t);
void setlinebuf(FILE *);
void *memset(void *, int, size_t);
typedef int wchar_t;
unsigned fputwc(wchar_t, FILE *);
int fputws(const wchar_t *, FILE *);
int fwprintf(FILE *, const wchar_t *, ...);
void _Exit(int);

static char big[10000];
static char small[64];
static char tiny[10];

/* What PATH holds now, read through a second stream. */
static void show_file(const char *when) {
  FILE *reader = fopen(PATH, "r");
  char text[64] = {0};
  size_t count = fread(text, 1, sizeof text - 1, reader);
  fclose(reader);
  fprintf(stderr, "%s: [%.*s]\n", when, (int)count, text);
}

int main(void) {
  memset(big, 'x', sizeof big - 1);
  if (MODE == 12) {
    setvbuf(stdout, small, 0, sizeof small);
    for (int i = 0; i < 40; i++)
      fputwc(L'w', stdout);
    fprintf(stderr, "E1\n");
    fputws(L"0123456789", stdout);
    fprintf(stderr, "E2\n");
    fwprintf(stdout, L"%ls|%d", L"formatted", 12345);
    fprintf(stderr, "E3\n");
    _Exit(0);
  }
  FILE *file = fopen(PATH, "w+");
  switch (MODE) {
  case 1: setvbuf(stdout, 0, 2, 0); break;
  case 2: setvbuf(stdout, 0, 1, 0); break;
  case 3: setvbuf(stdout, small, 0, sizeof small); break;
  case 4: setbuf(stdout, 0); break;
  case 5: setvbuf(stderr, 0, 0, 0); break;
  case 6: setvbuf(stderr, small, 1, 16); break;
  case 7: setlinebuf(stdout); setvbuf(file, 0, 2, 0); break;
  case 8: setvbuf(file, tiny, 0, sizeof tiny); break;
  case 9: printf("before "); setvbuf(stdout, 0, 2, 0); break;
  case 10: setbuffer(stdout, small, 20); setlinebuf(file); break;
  case 11: setvbuf(file, small, 1, 16); break;
  }
  printf("one\n");
  fprintf(stderr, "E1\n");
  printf("two ");
  fprintf(stderr, "E2 ");
  fputs("three\n", stdout);
  fprintf(stderr, "E3\n");
  fputs(big, stdout);
  fprintf(stderr, "E4\n");
  for (int i = 0; i < 5000; i++)
    putchar('y');
  fprintf(stderr, "E5\n");
  fwrite(big, 1, 5000, stdout);
  fputs("zz", stderr);
  fputs("\n", stderr);
  puts("p");
  printf("%s and %d\n", "formatted", 42);
  fprintf(stderr, "E6\n");
  fputs("hello world", file);
  long held = ftell(file);
  show_file("after fputs");
  fprintf(file, "%d\n%s", 12345, "tail");
  fprintf(stderr, "ftell %ld then %ld\n", held, ftell(file));
  show_file("after fprintf");
  fprintf(stderr, "bad mode %d\n", setvbuf(file, 0, 7, 0));
  fclose(file);
  show_file("after fclose");
  return 0;
}
