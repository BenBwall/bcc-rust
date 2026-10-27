/* When glibc's writes reach a file, and the stream orientation writes
   give: each stream holds what it writes in a buffer of the file's block
   size and writes it out when the buffer fills, at fflush, fflush(NULL)
   (the most recently opened stream first), fclose and before a seek or
   read; a write orients the stream even when it fails. Compared with the
   program built against glibc. Declarations are written out so that no
   system header is needed. */
typedef struct _IO_FILE FILE;
typedef unsigned long size_t;
extern FILE *stdout, *stderr;
int printf(const char *, ...);
int fprintf(FILE *, const char *, ...);
FILE *fopen(const char *, const char *);
FILE *freopen(const char *, const char *, FILE *);
int fclose(FILE *);
int fflush(FILE *);
int fputc(int, FILE *);
int fputs(const char *, FILE *);
size_t fwrite(const void *, size_t, size_t, FILE *);
size_t fread(void *, size_t, size_t, FILE *);
char *fgets(char *, int, FILE *);
int fgetc(FILE *);
int ungetc(int, FILE *);
int fseek(FILE *, long, int);
long ftell(FILE *);
int ferror(FILE *);
int remove(const char *);
char *tmpnam(char *);

static char name[32];
static char block[8192];

/* Replaces the file with `text`. */
static void reset(const char *text)
{
  FILE *file = fopen(name, "w");
  fputs(text, file);
  fclose(file);
}

/* How many bytes another stream reads from the file now. */
static size_t on_disk(void)
{
  FILE *reader = fopen(name, "rb");
  size_t count = fread(block, 1, sizeof block, reader);
  fclose(reader);
  return count;
}

/* The file's first byte. */
static int first_byte(void)
{
  FILE *reader = fopen(name, "rb");
  int byte = fgetc(reader);
  fclose(reader);
  return byte;
}

/* After ungetc and a SEEK_CUR of 1, where the stream is and what it reads:
   a stream that some operation oriented counts the pushed-back byte. */
static void report_orientation(FILE *file)
{
  int failed = fseek(file, 1, 1);
  long at = ftell(file);
  int next = fgetc(file);
  printf(" %d %ld %c", failed, at, next);
}

int main(void)
{
  tmpnam(name);
  /* A byte written stays in the buffer until fflush. */
  FILE *writer = fopen(name, "w");
  fputc('X', writer);
  size_t before = on_disk();
  fflush(writer);
  printf("held %lu %lu\n", before, on_disk());
  fclose(writer);
  /* A full buffer is written out when another byte comes; fwrite sends
     whole buffers straight to the file and keeps the rest; a small fputs
     stays in the buffer. */
  for (int i = 0; i < 8192; i++)
    block[i] = 'a';
  writer = fopen(name, "w");
  for (int i = 0; i < 4097; i++)
    fputc('a', writer);
  printf("overflow %lu", on_disk());
  fclose(writer);
  writer = fopen(name, "w");
  fwrite(block, 1, 5000, writer);
  printf(" %lu", on_disk());
  fclose(writer);
  writer = fopen(name, "w");
  fputs("abc", writer);
  printf(" %lu\n", on_disk());
  fclose(writer);
  /* Two streams writing the same byte: fflush(NULL) writes the most
     recently opened first, and fclose writes in the order of the calls. */
  reset("abcdef");
  FILE *first = fopen(name, "r+");
  FILE *second = fopen(name, "r+");
  fputc('A', first);
  fputc('B', second);
  fflush(0);
  int flushed = first_byte();
  fclose(first);
  fclose(second);
  reset("abcdef");
  first = fopen(name, "r+");
  second = fopen(name, "r+");
  fputc('A', first);
  fputc('B', second);
  fclose(second);
  fclose(first);
  printf("order %c %c\n", flushed, first_byte());
  /* A write that fails surfaces when the buffer is written out. */
  writer = fopen("/dev/full", "w");
  int put = fputc('X', writer);
  int error = ferror(writer);
  int status = fflush(writer);
  printf("full %d %d %d %d\n", put, error, status, fclose(writer));
  /* ftell counts the bytes held; an appending stream counts them from the
     file's end. */
  writer = fopen("/dev/null", "w");
  fputc('X', writer);
  long held = ftell(writer);
  fflush(writer);
  printf("tell %ld %ld ", held, ftell(writer));
  fclose(writer);
  reset("abcdef");
  writer = fopen(name, "a+");
  fputc('X', writer);
  FILE *other = fopen(name, "a");
  fputc('Y', other);
  fclose(other);
  printf("%ld\n", ftell(writer));
  fclose(writer);
  /* fgets with room for the terminator alone does not orient the stream;
     a write orients it even when the stream may not write. */
  reset("abcdef");
  FILE *file = fopen(name, "r");
  ungetc('X', file);
  char byte;
  fgets(&byte, 1, file);
  printf("orientation");
  report_orientation(file);
  fclose(file);
  for (int how = 0; how < 5; how++) {
    file = fopen(name, "r");
    ungetc('X', file);
    if (how == 0)
      fputc('Q', file);
    else if (how == 1)
      fputs("Q", file);
    else if (how == 2)
      fwrite("Q", 1, 1, file);
    else if (how == 3)
      fprintf(file, "Q");
    else
      fprintf(file, "%s", "");
    report_orientation(file);
    fclose(file);
  }
  printf("\n");
  /* printf orients a redirected stdout even when it formats nothing, and
     fails on one that may not write. */
  freopen(name, "r+", stdout);
  ungetc('X', stdout);
  printf("%s", "");
  int failed = fseek(stdout, 1, 1);
  long at = ftell(stdout);
  int next = fgetc(stdout);
  freopen(name, "r", stdout);
  int formatted = printf("%s", "");
  fprintf(stderr, "stdout %d %ld %c %d %d\n", failed, at, next, formatted, ferror(stdout));
  fclose(stdout);
  remove(name);
  return 0;
}
