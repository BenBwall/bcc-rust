/* bcc's bundled C library for wasm32-wasip1: a small <unistd.h>. */
#ifndef _BCC_WASI_UNISTD_H
#define _BCC_WASI_UNISTD_H
#include <stddef.h>
typedef long ssize_t;
#define STDIN_FILENO 0
#define STDOUT_FILENO 1
#define STDERR_FILENO 2
ssize_t write(int fd, const void *buf, size_t count);
ssize_t read(int fd, void *buf, size_t count);
_Noreturn void _exit(int status);
#endif
