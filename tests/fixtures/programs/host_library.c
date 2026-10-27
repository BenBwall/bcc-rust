/* C library functions the interpreter's host provides, printed so that the
   interpreter's output can be compared with the program built against
   glibc: streams and files, formatted input and output through va_list,
   qsort and bsearch with interpreted comparisons, strings, wide strings,
   character classes and numeric conversion. Declarations are written out
   so that no system header is needed. */
typedef struct _IO_FILE FILE;
typedef __builtin_va_list va_list;
typedef unsigned long size_t;
typedef int wchar_t;
extern FILE *stdin, *stdout, *stderr;
int printf(const char *, ...);
int fprintf(FILE *, const char *, ...);
int vprintf(const char *, va_list);
int vfprintf(FILE *, const char *, va_list);
int vsprintf(char *, const char *, va_list);
int vsnprintf(char *, size_t, const char *, va_list);
int sscanf(const char *, const char *, ...) __asm__("__isoc99_sscanf");
int fscanf(FILE *, const char *, ...) __asm__("__isoc99_fscanf");
FILE *fopen(const char *, const char *);
FILE *freopen(const char *, const char *, FILE *);
int fclose(FILE *);
int fflush(FILE *);
int fputs(const char *, FILE *);
int fputc(int, FILE *);
int putc(int, FILE *);
size_t fwrite(const void *, size_t, size_t, FILE *);
size_t fread(void *, size_t, size_t, FILE *);
char *fgets(char *, int, FILE *);
int fgetc(FILE *);
int ungetc(int, FILE *);
int feof(FILE *);
int ferror(FILE *);
void clearerr(FILE *);
int fseek(FILE *, long, int);
long ftell(FILE *);
void rewind(FILE *);
int remove(const char *);
char *tmpnam(char *);
void qsort(void *, size_t, size_t, int (*)(const void *, const void *));
void *bsearch(const void *, const void *, size_t, size_t, int (*)(const void *, const void *));
char *strcat(char *, const char *);
char *strncat(char *, const char *, size_t);
char *strstr(const char *, const char *);
char *strchr(const char *, int);
char *strrchr(const char *, int);
void *memchr(const void *, int, size_t);
size_t strspn(const char *, const char *);
size_t strcspn(const char *, const char *);
char *strpbrk(const char *, const char *);
size_t strnlen(const char *, size_t);
char *strdup(const char *);
char *strndup(const char *, size_t);
char *stpcpy(char *, const char *);
char *strtok(char *, const char *);
size_t strlen(const char *);
int strcmp(const char *, const char *);
void free(void *);
int atoi(const char *);
long atol(const char *);
long strtol(const char *, char **, int);
unsigned long strtoul(const char *, char **, int);
long long strtoll(const char *, char **, int);
unsigned long long strtoull(const char *, char **, int);
int isalpha(int), isdigit(int), isspace(int), ispunct(int), isxdigit(int), isupper(int);
int tolower(int), toupper(int);
const unsigned short **__ctype_b_loc(void);
const int **__ctype_tolower_loc(void);
size_t wcslen(const wchar_t *);
wchar_t *wcscpy(wchar_t *, const wchar_t *);
wchar_t *wcscat(wchar_t *, const wchar_t *);
int wcscmp(const wchar_t *, const wchar_t *);
int wcsncmp(const wchar_t *, const wchar_t *, size_t);
wchar_t *wcschr(const wchar_t *, wchar_t);
wchar_t *wcsrchr(const wchar_t *, wchar_t);
wchar_t *wmemset(wchar_t *, wchar_t, size_t);
int wmemcmp(const wchar_t *, const wchar_t *, size_t);
double _Complex conj(double _Complex);
float _Complex conjf(float _Complex);
long double _Complex conjl(long double _Complex);
double _Complex cproj(double _Complex);
double cabs(double _Complex);
float cabsf(float _Complex);
double carg(double _Complex);

static int calls;

int sprintf(char *, const char *, ...);

struct big {
  int key;
  int tag;
  char padding[32];
};

static const void *trace_base;
static size_t trace_size;
static char trace[512];
static int traced;

/* Records which elements it compares, as offsets from `trace_base`, and
   orders them by their first int. */
static int traced_compare(const void *a, const void *b)
{
  long i = ((const char *)a - (const char *)trace_base) / (long)trace_size;
  long j = ((const char *)b - (const char *)trace_base) / (long)trace_size;
  traced += sprintf(trace + traced, " %ld%ld", i, j);
  int x = *(const int *)a, y = *(const int *)b;
  return (x > y) - (x < y);
}

static int by_value(const void *a, const void *b)
{
  calls++;
  int x = *(const int *)a, y = *(const int *)b;
  return (x > y) - (x < y);
}

struct pair { int key; const char *name; };

/* Orders only by key, so equal keys show that the sort is stable. */
static int by_key(const void *a, const void *b)
{
  return ((const struct pair *)a)->key - ((const struct pair *)b)->key;
}

static int by_string(const void *a, const void *b)
{
  return strcmp(*(const char *const *)a, *(const char *const *)b);
}

static void report(FILE *stream, const char *format, ...)
{
  va_list list;
  __builtin_va_start(list, format);
  vfprintf(stream, format, list);
  __builtin_va_end(list);
  __builtin_va_start(list, format);
  vprintf(format, list);
  __builtin_va_end(list);
  char text[16];
  __builtin_va_start(list, format);
  int length = vsnprintf(text, sizeof text, format, list);
  __builtin_va_end(list);
  printf("[%d %s]\n", length, text);
}

/* Writes `count` copies of `fill` and then `tail` to a new file `name`. */
static void make_file(const char *name, int fill, int count, const char *tail)
{
  FILE *file = fopen(name, "wb");
  for (int i = 0; i < count; i++)
    fputc(fill, file);
  fputs(tail, file);
  fclose(file);
}

/* glibc's read buffering: a buffer of the file's block size, refilled when
   empty; reads of at least a buffer go straight into the array; ungetc
   steps back over the byte just read from the buffer, else pushes into a
   separate area that any seek discards; and a seek refills the buffer up to
   the new position from the block boundary below it. */
static void buffering(void)
{
  static char block[8192];
  char name[32], other[32];
  tmpnam(name);
  tmpnam(other);
  /* A set end-of-file indicator stops refills but not a read straight into
     a large array. */
  make_file(name, 'x', 3, "");
  FILE *file = fopen(name, "rb");
  while (fgetc(file) != -1)
    ;
  FILE *more = fopen(name, "ab");
  for (int i = 0; i < 4096; i++)
    fputc('q', more);
  fclose(more);
  size_t got = fread(block, 1, 4096, file);
  int eof = !!feof(file);
  size_t small = fread(block, 1, 10, file);
  int next = fgetc(file);
  printf("buffer eof %lu %d %lu %d\n", got, eof, small, next);
  fclose(file);
  /* Bytes read straight into the array are not in the buffer, so ungetc
     pushes the last of them into the area a failed seek discards. */
  make_file(name, 'a', 4096, "B");
  file = fopen(name, "rb");
  fread(block, 1, 4096, file);
  ungetc(block[4095], file);
  int failed = fseek(file, -1, 0);
  long at = ftell(file);
  next = fgetc(file);
  printf("direct %d %ld %c\n", failed, at, next);
  fclose(file);
  /* After a seek off a block boundary the buffer holds the bytes before the
     position; a seek within the buffer moves within it; a seek relative to
     an offset glibc does not know yet empties it. */
  make_file(name, '0', 0, "0123456789");
  file = fopen(name, "rb");
  fseek(file, 1, 0);
  ungetc('0', file);
  failed = fseek(file, -1, 0);
  next = fgetc(file);
  printf("seek %d %c ", failed, next);
  fgetc(file);
  fgetc(file);
  fseek(file, 5, 0);
  ungetc('4', file);
  failed = fseek(file, -1, 0);
  next = fgetc(file);
  printf("%d %c ", failed, next);
  fclose(file);
  file = fopen(name, "rb");
  fgetc(file);
  fseek(file, 2, 1);
  ungetc('2', file);
  failed = fseek(file, -1, 0);
  next = fgetc(file);
  printf("%d %c\n", failed, next);
  fclose(file);
  /* ftell counts the pushback area only once there is a buffer, and then
     subtracts the buffer's unread bytes behind it without a sign check. */
  file = fopen(name, "rb");
  ungetc('Z', file);
  at = ftell(file);
  next = fgetc(file);
  printf("tell %ld %c ", at, next);
  ungetc('Y', file);
  printf("%ld ", ftell(file));
  next = fgetc(file);
  int first = fgetc(file);
  ungetc('W', file);
  printf("%c %c %ld ", next, first, ftell(file));
  ungetc('V', file);
  ungetc('U', file);
  printf("%ld", ftell(file));
  for (int i = 0; i < 4; i++)
    printf(" %c", fgetc(file));
  printf("\n");
  fclose(file);
  /* A stream that only appends starts at the end, one that also reads at
     the beginning. */
  file = fopen(name, "a");
  at = ftell(file);
  fclose(file);
  file = fopen(name, "a+");
  printf("append %ld %ld\n", at, ftell(file));
  fclose(file);
  /* The buffer keeps what it read when another stream rewrites the file;
     fflush gives it back, so the next read sees the new bytes. */
  make_file(other, 'a', 0, "abc");
  file = fopen(other, "rb");
  fgetc(file);
  FILE *writer = fopen(other, "r+b");
  fseek(writer, 1, 0);
  fputc('X', writer);
  fclose(writer);
  next = fgetc(file);
  fseek(file, 0, 0);
  fgetc(file);
  writer = fopen(other, "r+b");
  fseek(writer, 1, 0);
  fputc('Y', writer);
  fclose(writer);
  int flushed = fflush(file);
  printf("stale %c %d %c\n", next, flushed, fgetc(file));
  fclose(file);
  remove(name);
  remove(other);
}

int main(void)
{
  int values[] = {5, -3, 9, 0, 9, 2, -8, 7, 1, 4, 4, 6};
  qsort(values, 12, sizeof values[0], by_value);
  for (int i = 0; i < 12; i++)
    printf("%d ", values[i]);
  printf("(%d comparisons)\n", calls);
  int wanted = 7, missing = 3;
  int *found = bsearch(&wanted, values, 12, sizeof values[0], by_value);
  printf("bsearch %ld %p\n", (long)(found - values),
         bsearch(&missing, values, 12, sizeof values[0], by_value));
  struct pair pairs[] = {{2, "b1"}, {1, "a1"}, {2, "b2"}, {0, "z"}, {1, "a2"}, {2, "b3"}};
  qsort(pairs, 6, sizeof pairs[0], by_key);
  for (int i = 0; i < 6; i++)
    printf("%s ", pairs[i].name);
  const char *words[] = {"pear", "apple", "fig", "banana", "cherry"};
  qsort(words, 5, sizeof words[0], by_string);
  printf("| %s %s %s %s %s\n", words[0], words[1], words[2], words[3], words[4]);

  char buffer[64] = "abc";
  strcat(buffer, "def");
  strncat(buffer, "ghijkl", 3);
  char *end = stpcpy(buffer + strlen(buffer), "!xy");
  printf("%s %ld %s %s %s %s\n", buffer, (long)(end - buffer), strstr(buffer, "ef"),
         strchr(buffer, 'g'), strrchr(buffer, 'x'), strpbrk(buffer, "hz!"));
  printf("%lu %lu %lu %d %d\n", strspn(buffer, "abc"), strcspn(buffer, "fg"),
         strnlen(buffer, 4), strstr(buffer, "zz") == 0, memchr(buffer, 'e', 3) == 0);
  char *copy = strdup(buffer), *prefix = strndup(buffer, 2);
  printf("%s %s\n", copy, prefix);
  free(copy);
  free(prefix);
  char sentence[] = "  one, two;three  ,four";
  for (char *token = strtok(sentence, " ,;"); token; token = strtok(0, " ,;"))
    printf("<%s>", token);
  printf("\n");

  char *rest;
  long parsed = strtol("0x1F!", &rest, 0);
  printf("%d %ld %ld %lu %lld %llu %s\n", atoi("  -42xyz"), atol("+77"), parsed,
         strtoul("-1", 0, 10), strtoll("077", 0, 0), strtoull("zz", 0, 36), rest);
  long octal = strtol("  12", &rest, 8);
  printf("%ld %ld %ld [%s]\n", strtol("99999999999999999999", 0, 10),
         strtol("-99999999999999999999", 0, 10), octal, rest);
  for (int c = -1; c < 130; c += 13)
    printf("%d:%d%d%d%d%d%d%c%c ", c, !!isalpha(c), !!isdigit(c), !!isspace(c), !!ispunct(c),
           !!isxdigit(c), !!isupper(c), c > 31 && c < 127 ? tolower(c) : '.',
           c > 31 && c < 127 ? toupper(c) : '.');
  printf("\n%x %x %x %d %d\n", (*__ctype_b_loc())['A'], (*__ctype_b_loc())[' '],
         (*__ctype_b_loc())[200], (*__ctype_tolower_loc())['Q'], (*__ctype_tolower_loc())[-1]);

  wchar_t wide[32];
  wcscpy(wide, L"wide");
  wcscat(wide, L" chars");
  printf("%lu %d %d %ld %ld", wcslen(wide), wcscmp(wide, L"wide"), wcsncmp(wide, L"wider", 4),
         (long)(wcschr(wide, L'c') - wide), (long)(wcsrchr(wide, L'e') - wide));
  wmemset(wide, L'z', 3);
  wchar_t negative[] = {-5, 0};
  printf(" %d %d\n", wmemcmp(wide, L"zzze", 4), wcscmp(negative, L"a"));

  int number = 0, count = 0;
  unsigned hex = 0;
  char word[16] = "", set[16] = "";
  double real = 0;
  float single = 0;
  int scanned = sscanf("  -17 0x1fz word123 [abc] 2.5e3 -0.125", "%d %x%*c %5s%n%*s %*c%[a-c]] %lf %f",
                       &number, &hex, word, &count, set, &real, &single);
  printf("%d %d %x %s %d %s %g %g\n", scanned, number, hex, word, count, set, real, single);
  printf("%d %d\n", sscanf("", "%d", &number), sscanf("x", "%d", &number));

  char name[32];
  tmpnam(name);
  FILE *file = fopen(name, "w+");
  fprintf(file, "line %d\n", 1);
  fputs("second line\n", file);
  fputc('3', file);
  putc('\n', file);
  fwrite("bytes\n", 1, 6, file);
  report(file, "%s=%d\n", "reported", 42);
  printf("ftell %ld\n", ftell(file));
  rewind(file);
  char line[32];
  while (fgets(line, sizeof line, file))
    printf("read: %s", line);
  printf("eof %d error %d\n", !!feof(file), !!ferror(file));
  fseek(file, 5, 0);
  int first = fgetc(file);
  ungetc('X', file);
  printf("%c%c ", first, fgetc(file));
  fseek(file, 0, 0);
  printf("%d ", fscanf(file, "%s %d", word, &number));
  printf("%s %d\n", word, number);
  char raw[8] = "";
  fseek(file, -7, 2);
  printf("%lu %.6s\n", fread(raw, 1, 7, file), raw);
  /* SEEK_CUR counts from before a pushed-back byte, then drops it. */
  fseek(file, 3, 0);
  ungetc(fgetc(file), file);
  fseek(file, 2, 1);
  printf("seek %c ", fgetc(file));
  /* Empty requests and fgets limits without room read and write nothing. */
  printf("%lu %lu ", fwrite("x", 0, 3, file), fwrite("x", 3, 0, file));
  char untouched[4] = "abc";
  printf("%d %d %s ", fgets(untouched, 0, file) == 0, fgets(untouched, -1, file) == 0, untouched);
  printf("%d [%s]\n", fgets(untouched, 1, file) == untouched, untouched);
  /* Seeks out of range or with an unknown origin fail; a byte pushed back
     at the start has no position. */
  rewind(file);
  ungetc(fgetc(file), file);
  printf("ftell %ld ", ftell(file));
  printf("%d ", fseek(file, -9223372036854775807L - 1, 1));
  printf("%d %d ", fseek(file, -1, 0), fseek(file, 0, 7));
  printf("%c ", fgetc(file));
  rewind(file);
  ungetc('Z', file);
  printf("%ld %c\n", ftell(file), fgetc(file));
  /* A seek that fails discards a byte pushed back apart from what was
     read, but keeps one that steps back over the byte just read, and keeps
     the end-of-file indicator. */
  rewind(file);
  ungetc('Z', file);
  int failed = fseek(file, -1, 0);
  printf("pushback %d %c ", failed, fgetc(file));
  ungetc('Q', file);
  failed = fseek(file, -1, 1);
  printf("%d %c ", failed, fgetc(file));
  ungetc('i', file);
  failed = fseek(file, -100, 2);
  printf("%d %c ", failed, fgetc(file));
  fgetc(file);
  ungetc('X', file);
  fgetc(file);
  ungetc('X', file);
  failed = fseek(file, -1, 0);
  printf("%d %c ", failed, fgetc(file));
  ungetc('Y', file);
  failed = fseek(file, 0, 7);
  printf("%d %c\n", failed, fgetc(file));
  while (fgetc(file) != -1)
    ;
  failed = fseek(file, -1000, 1);
  printf("eof %d %d ", failed, !!feof(file));
  /* End-of-file stays set until it is cleared, even once the file grows. */
  FILE *more = fopen(name, "a");
  fputs("!", more);
  fclose(more);
  int stuck = fgetc(file);
  clearerr(file);
  printf("%d %c\n", stuck, fgetc(file));
  fclose(file);
  /* "x" refuses an existing file instead of truncating it. */
  FILE *exclusive = fopen(name, "wx");
  printf("wx existing %d ", exclusive == 0);
  int removed = remove(name);
  int again = remove(name);
  printf("remove %d %d ", removed, again);
  exclusive = fopen(name, "wx");
  printf("wx new %d\n", exclusive != 0);
  fclose(exclusive);
  remove(name);
  printf("fopen missing %d\n", fopen(name, "r") == 0);
  buffering();

  /* Comparison traces: offsets of the compared elements from the base,
     into the array as it is merged for small elements, and where the
     elements started for those over 32 bytes, which glibc sorts through
     pointers. */
  int small[] = {5, 3, 8, 1, 9, 2, 7, 4};
  trace_base = small;
  trace_size = sizeof small[0];
  qsort(small, 8, sizeof small[0], traced_compare);
  printf("small %s:", trace);
  for (int i = 0; i < 8; i++)
    printf(" %d", small[i]);
  struct big big[8];
  for (int i = 0; i < 8; i++) {
    big[i].key = small[(i * 5) % 8] * 10 + i % 3;
    big[i].tag = i;
  }
  trace_base = big;
  trace_size = sizeof big[0];
  traced = 0;
  trace[0] = 0;
  qsort(big, 8, sizeof big[0], traced_compare);
  printf("\nbig %s:", trace);
  for (int i = 0; i < 8; i++)
    printf(" %d/%d", big[i].key, big[i].tag);
  printf("\n");

  volatile double _Complex z = 3.0 + 4.0i;
  double _Complex c = conj(z), p = cproj(z), q = cproj(z / 0.0);
  float _Complex f = conjf((float _Complex)z);
  long double _Complex l = conjl(z);
  printf("%g%+gi %g%+gi %g%+gi %g%+gi %Lg%+Lgi %g %g %g\n", __real__ c, __imag__ c, __real__ p,
         __imag__ p, __real__ q, __imag__ q, (double)__real__ f, (double)__imag__ f, __real__ l,
         __imag__ l, cabs(z), (double)cabsf((float _Complex)z), carg(z));

  fprintf(stdout, "to stdout %d\n", 1);
  fprintf(stderr, "to stderr %d\n", 2);
  fflush(stdout);
  report(stdout, "%c%c%c\n", 'a', 'b', 'c');
  return 0;
}
