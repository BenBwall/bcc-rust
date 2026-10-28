/* bcc's bundled C library for wasm32-wasip1: <stdio.h>. */
#ifndef _BCC_WASI_STDIO_H
#define _BCC_WASI_STDIO_H
#include <stddef.h>
#include <stdarg.h>

typedef struct __bcc_file FILE;
typedef long long fpos_t;

extern FILE *const stdin;
extern FILE *const stdout;
extern FILE *const stderr;
#define stdin stdin
#define stdout stdout
#define stderr stderr

#define EOF (-1)
#define BUFSIZ 1024
#define FILENAME_MAX 4096
#define FOPEN_MAX 8
#define L_tmpnam 20
#define TMP_MAX 238328
#define SEEK_SET 0
#define SEEK_CUR 1
#define SEEK_END 2
#define _IOFBF 0
#define _IOLBF 1
#define _IONBF 2

int printf(const char *restrict format, ...);
int fprintf(FILE *restrict stream, const char *restrict format, ...);
int sprintf(char *restrict s, const char *restrict format, ...);
int snprintf(char *restrict s, size_t n, const char *restrict format, ...);
int vprintf(const char *restrict format, va_list ap);
int vfprintf(FILE *restrict stream, const char *restrict format, va_list ap);
int vsprintf(char *restrict s, const char *restrict format, va_list ap);
int vsnprintf(char *restrict s, size_t n, const char *restrict format, va_list ap);
int sscanf(const char *restrict s, const char *restrict format, ...);
int vsscanf(const char *restrict s, const char *restrict format, va_list ap);
int scanf(const char *restrict format, ...);
int fscanf(FILE *restrict stream, const char *restrict format, ...);

int fputc(int c, FILE *stream);
int putc(int c, FILE *stream);
int putchar(int c);
int fputs(const char *restrict s, FILE *restrict stream);
int puts(const char *s);
int fgetc(FILE *stream);
int getc(FILE *stream);
int getchar(void);
int ungetc(int c, FILE *stream);
char *fgets(char *restrict s, int n, FILE *restrict stream);
size_t fread(void *restrict ptr, size_t size, size_t nmemb, FILE *restrict stream);
size_t fwrite(const void *restrict ptr, size_t size, size_t nmemb, FILE *restrict stream);
int fflush(FILE *stream);
int setvbuf(FILE *restrict stream, char *restrict buf, int mode, size_t size);
void setbuf(FILE *restrict stream, char *restrict buf);
int feof(FILE *stream);
int ferror(FILE *stream);
void clearerr(FILE *stream);
void perror(const char *s);
int fclose(FILE *stream);
FILE *fopen(const char *restrict filename, const char *restrict mode);
int remove(const char *filename);
int rename(const char *old, const char *new);

#endif
