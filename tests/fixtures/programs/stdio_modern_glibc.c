/* stdio behaviour that changed between glibc 2.39 and 2.43, which the
   interpreter follows in its 2.43 form: fflush(NULL) now gives back what
   streams that read have read ahead, and fflush of a stream drops its
   pushback area and seeks back over it as well as over the buffer. The
   expected output is in stdio_modern_glibc.expected. Declarations are
   written out so that no system header is needed. */
typedef struct _IO_FILE FILE;
int printf(const char *, ...);
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
  remove(name);
  return 0;
}
