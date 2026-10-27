/* Every sequence of four operations on a wide stream, with ftell and
   errno after each and the next character at the end: R reads, U pushes
   the last character read back, Z pushes another character back, F
   flushes, and P seeks to where ftell last said. In the "C" locale and in
   UTF-8, through the default buffer and small setvbuf buffers. glibc
   aborts a seek from the current position while the backup area is
   read, so there is none here, and aborts or hangs on some seeks in the
   "C" locale through buffers of 3 to 5 bytes, which are left out. glibc
   2.39 and 2.43 print the same. */

#include <errno.h>
#include <locale.h>
#include <stdio.h>
#include <wchar.h>

int main(void) {
  char name[L_tmpnam];
  tmpnam(name);
  static const char ops[] = "RUZFP";
  for (int utf = 0; utf < 2; utf++) {
    setlocale(LC_ALL, utf ? "C.UTF-8" : "C");
    FILE *w = fopen(name, "w");
    fputs(utf ? "a\xc3\xa9\xe2\x82\xac" "bcdefgh" : "abcdefghijk", w);
    fclose(w);
    int sizes[2][5] = {{0, 1, 2, 0, 0}, {0, 1, 2, 3, 5}};
    for (int s = 0; s < (utf ? 5 : 3); s++)
      for (int code = 0; code < 5 * 5 * 5 * 5; code++) {
        FILE *f = fopen(name, "r");
        char buffer[8];
        if (sizes[utf][s]) setvbuf(f, buffer, _IOFBF, sizes[utf][s]);
        printf("u%d s%d ", utf, sizes[utf][s]);
        wint_t last = L'?';
        long told = 0;
        for (int i = 0, c = code; i < 4; i++, c /= 5) {
          char op = ops[c % 5];
          long r = 0;
          errno = 0;
          switch (op) {
          case 'R': { wint_t got = fgetwc(f); if (got != WEOF) last = got; r = (long)got; break; }
          case 'U': r = (long)ungetwc(last, f); break;
          case 'Z': r = (long)ungetwc(L'Z', f); break;
          case 'F': r = fflush(f); break;
          default: r = fseek(f, told, SEEK_SET); break;
          }
          int e = errno;
          errno = 0;
          long t = ftell(f);
          int te = errno;
          if (t >= 0) told = t;
          printf("%c%ld/%d:%ld/%d ", op, r, e, t, te);
        }
        wint_t next = fgetwc(f);
        printf("next %ld\n", (long)next);
        fclose(f);
      }
  }
  remove(name);
  return 0;
}
