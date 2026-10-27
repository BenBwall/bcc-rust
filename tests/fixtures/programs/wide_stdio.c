/* Wide character streams: orientation and fwide, wide output to a file
   (fputwc, fputws, fwprintf with %ls, %s, %lc and field widths) read back
   with fgetwc, fgetws and ungetwc and as bytes; swprintf, including output
   that does not fit; swscanf and fwscanf with the narrow and wide
   conversions; narrow printf and sscanf with %lc and %ls; and in the "C"
   locale a byte beyond ASCII as an encoding error, while "C.UTF-8" reads
   and writes UTF-8. Text written in the "C" locale stays ASCII, since glibc
   versions disagree on what they write for other characters. The
   interpreter must print what the program built against glibc prints.
   Declarations are written out so that no system header is needed. */
typedef struct _IO_FILE FILE;
typedef unsigned long size_t;
typedef int wchar_t;
typedef unsigned int wint_t;
extern FILE *stdout, *stderr;
int *__errno_location(void);
#define errno (*__errno_location())
int printf(const char *, ...);
int snprintf(char *, size_t, const char *, ...);
int sscanf(const char *, const char *, ...) __asm__("__isoc99_sscanf");
FILE *fopen(const char *, const char *);
int fclose(FILE *);
int fputs(const char *, FILE *);
int fgetc(FILE *);
int ferror(FILE *);
int feof(FILE *);
void rewind(FILE *);
char *tmpnam(char *);
int remove(const char *);
char *setlocale(int, const char *);
int fwide(FILE *, int);
wint_t fputwc(wchar_t, FILE *);
int fputws(const wchar_t *, FILE *);
wint_t fgetwc(FILE *);
wchar_t *fgetws(wchar_t *, int, FILE *);
wint_t ungetwc(wint_t, FILE *);
int fwprintf(FILE *, const wchar_t *, ...);
int swprintf(wchar_t *, size_t, const wchar_t *, ...);
int swscanf(const wchar_t *, const wchar_t *, ...) __asm__("__isoc99_swscanf");
int fwscanf(FILE *, const wchar_t *, ...) __asm__("__isoc99_fwscanf");

#define WEOF ((wint_t)-1)

/* Prints the code of each wide character the file holds, then its state. */
static void dump_wide(FILE *file) {
  wint_t character;
  rewind(file);
  errno = 0;
  while ((character = fgetwc(file)) != WEOF)
    printf("%x ", character);
  printf("| eof %d error %d errno %d\n", feof(file), ferror(file), errno);
}

int main(void) {
  char *name = tmpnam(0);
  FILE *file = fopen(name, "w+");
  printf("fwide new %d, ask %d\n", fwide(file, 0), fwide(file, 0));
  printf("fwide wide %d, then byte %d\n", fwide(file, 5), fwide(file, -1));
  printf("fputwc %d\n", (int)fputwc(L'A', file));
  printf("fputws %d\n", fputws(L"bc\n", file));
  int count = fwprintf(file, L"[%5d|%-4ls|%s|%lc|%.2ls]\n", 42, L"wi", "na", L'z', L"xyz");
  printf("fwprintf %d\n", count);
  dump_wide(file);
  rewind(file);
  wchar_t line[16];
  printf("fgetws [%ls]", fgetws(line, 16, file));
  wint_t next = fgetwc(file);
  printf("ungetwc %d, again %d\n", (int)ungetwc(next, file), (int)fgetwc(file));
  printf("fgetws short [%ls]\n", fgetws(line, 4, file));
  printf("ungetwc WEOF %d\n", (int)ungetwc(WEOF, file));
  fclose(file);

  file = fopen(name, "r");
  int bytes = 0, byte;
  while ((byte = fgetc(file)) != -1)
    bytes += byte;
  printf("byte sum %d, fwide %d\n", bytes, fwide(file, 1));
  fclose(file);

  wchar_t buffer[8];
  errno = 0;
  count = swprintf(buffer, 8, L"%d-%ls", 12345, L"abcdef");
  printf("swprintf too long %d errno %d [%ls]\n", count, errno, buffer);
  count = swprintf(buffer, 8, L"%x%c", 255, 'q');
  printf("swprintf %d [%ls]\n", count, buffer);

  int number = 0;
  unsigned hex = 0;
  double real = 0;
  char narrow[8];
  wchar_t wide[8], single = 0;
  count = swscanf(L"  -17 0x1f 2.5e1 abc Z rest", L"%d %x %lf %7s %lc %ls", &number, &hex,
                  &real, narrow, &single, wide);
  printf("swscanf %d: %d %u %g %s %lc %ls\n", count, number, hex, real, narrow, single, wide);
  count = swscanf(L"12abc", L"%d%n", &number, &hex);
  printf("swscanf %d %d %u\n", count, number, hex);
  printf("swscanf empty %d\n", swscanf(L"", L"%d", &number));
  count = sscanf("ab cd", "%ls %l[a-z]", wide, line);
  printf("sscanf %d [%ls] [%ls]\n", count, wide, line);
  char text[32];
  count = snprintf(text, sizeof text, "%ls|%lc|%3.1ls|", L"wide", L'c', L"xy");
  printf("snprintf %d [%s]\n", count, text);

  file = fopen(name, "w");
  fputs("7 words here\nh\xc3\xa9!\n", file);
  fclose(file);
  file = fopen(name, "r");
  count = fwscanf(file, L"%d %ls", &number, wide);
  printf("fwscanf %d %d [%ls] next %x\n", count, number, wide, fgetwc(file));
  fgetws(line, 16, file);
  dump_wide(file);
  errno = 0;
  count = snprintf(text, sizeof text, "%ls", L"\xe9");
  printf("narrow %%ls of e9: %d errno %d\n", count, errno);
  fclose(file);

  printf("locale %s\n", setlocale(6, "C.UTF-8"));
  file = fopen(name, "r");
  dump_wide(file);
  fclose(file);
  file = fopen(name, "w+");
  fwprintf(file, L"%ls %lc %s %3ls|", L"\xe9t\xe9", 0x20ac, "h\xc3\xa9", L"\xfc");
  dump_wide(file);
  fclose(file);
  count = snprintf(text, sizeof text, "%ls|%lc", L"\xe9", 0x20ac);
  printf("snprintf %d:", count);
  for (int i = 0; i < count; i++)
    printf(" %02x", (unsigned char)text[i]);
  printf("\n");
  remove(name);
  return 0;
}
