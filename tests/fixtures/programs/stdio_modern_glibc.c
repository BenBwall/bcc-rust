/* stdio behaviour that changed between glibc 2.39 and 2.43, which the
   interpreter follows in its 2.43 form. glibc 2.42 made fflush(NULL) give
   back what streams that read have read ahead, and fflush of a stream drop
   its pushback area and seek back over it as well as over the buffer
   (commits 94251ae99eda and 377e9733b50c, bugs 32369 and 5994). glibc 2.41
   made a seek count and drop the pushback area of an oriented stream whose
   buffer was never allocated (commit 3e1d8d1d1dca, bug 27821: before it,
   _IO_have_backup tested _IO_save_base, which ungetc on such a stream
   leaves null). The expected output is in stdio_modern_glibc.expected.
   Declarations are written out so that no system header is needed. */
typedef struct _IO_FILE FILE;
typedef unsigned long size_t;
extern FILE *stdout, *stderr;
int printf(const char *, ...);
int fprintf(FILE *, const char *, ...);
FILE *freopen(const char *, const char *, FILE *);
size_t fwrite(const void *, size_t, size_t, FILE *);
FILE *fopen(const char *, const char *);
int fclose(FILE *);
int fflush(FILE *);
int fputs(const char *, FILE *);
int fputc(int, FILE *);
int fgetc(FILE *);
int ungetc(int, FILE *);
int fseek(FILE *, long, int);
long ftell(FILE *);
int remove(const char *);
char *tmpnam(char *);

/* After ungetc and a SEEK_CUR of 1, where the stream is and what it reads:
   since glibc 2.41 a stream that some operation oriented counts the
   pushed-back byte even without a buffer; before, the seek dropped it
   uncounted, landing at 1 and reading "b". */
static void report_orientation(FILE *file, FILE *to)
{
  int failed = fseek(file, 1, 1);
  long at = ftell(file);
  int next = fgetc(file);
  fprintf(to, " %d %ld %c", failed, at, next);
}

int main(void)
{
  char name[32];
  tmpnam(name);
  FILE *writer = fopen(name, "wb");
  fputs("abc", writer);
  fclose(writer);
  /* glibc 2.39 keeps the stale "b" in the reader's buffer. */
  FILE *reader = fopen(name, "rb");
  int first = fgetc(reader);
  writer = fopen(name, "r+b");
  fseek(writer, 1, 0);
  fputc('X', writer);
  fclose(writer);
  int flushed = fflush(0);
  int second = fgetc(reader);
  printf("flush all %c %d %c\n", first, flushed, second);
  /* glibc 2.39 seeks back over the pushed-back byte alone and keeps the
     buffer, so it reads the rest of the buffer and then that byte again. */
  fseek(reader, 0, 0);
  first = fgetc(reader);
  ungetc('Z', reader);
  flushed = fflush(reader);
  long at = ftell(reader);
  printf("flush pushback %c %d %ld", first, flushed, at);
  for (int i = 0; i < 4; i++)
    printf(" %d", fgetc(reader));
  printf("\n");
  fclose(reader);
  /* A write orients the stream even when it fails before the stream has a
     buffer, as does a printf that formats nothing. */
  writer = fopen(name, "w");
  fputs("abcdef", writer);
  fclose(writer);
  printf("orientation");
  for (int how = 0; how < 5; how++) {
    FILE *file = fopen(name, "r");
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
    report_orientation(file, stdout);
    fclose(file);
  }
  printf("\n");
  /* So does printf to a redirected stdout that has no buffer yet. */
  freopen(name, "r+", stdout);
  ungetc('X', stdout);
  printf("%s", "");
  fprintf(stderr, "stdout");
  report_orientation(stdout, stderr);
  fprintf(stderr, "\n");
  fclose(stdout);
  remove(name);
  return 0;
}
