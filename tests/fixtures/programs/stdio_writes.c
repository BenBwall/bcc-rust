/* When glibc's writes reach a file: each stream holds what it writes in a
   buffer of the file's block size and writes it out when the buffer fills,
   at fflush, fflush(NULL) (the most recently opened stream first), fclose,
   freopen and before a seek or read. Compared with the program built
   against glibc; the orientation cases that glibc 2.41 changed are in
   stdio_modern_glibc.c. Declarations are written out so that no system
   header is needed. */
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
  /* fgets with room for the terminator alone does not orient the stream,
     so a SEEK_CUR after ungetc drops the pushed-back byte uncounted. */
  reset("abcdef");
  FILE *file = fopen(name, "r");
  ungetc('X', file);
  char byte;
  fgets(&byte, 1, file);
  int failed = fseek(file, 1, 1);
  long at = ftell(file);
  int next = fgetc(file);
  printf("fgets %d %ld %c\n", failed, at, next);
  fclose(file);
  /* freopen closes the stream, writing out what it holds, before it opens
     the file: reopening its own file for writing truncates those bytes,
     and a mode it refuses still closes the stream. */
  writer = fopen(name, "w");
  fputs("old", writer);
  writer = freopen(name, "w", writer);
  fputs("N", writer);
  fclose(writer);
  printf("reopen %lu %c", on_disk(), first_byte());
  writer = fopen(name, "w");
  fputs("old", writer);
  int refused = freopen(name, "q", writer) == 0;
  printf(" %d %lu\n", refused, on_disk());
  /* printf fails on a redirected stdout that may not write, even when it
     formats nothing. */
  freopen(name, "r", stdout);
  int formatted = printf("%s", "");
  fprintf(stderr, "stdout %d %d\n", formatted, ferror(stdout));
  fclose(stdout);
  remove(name);
  return 0;
}
