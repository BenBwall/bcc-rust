/* bcc's bundled C library for wasm32-wasip1: <errno.h>, with WASI's
   error numbers. */
#ifndef _BCC_WASI_ERRNO_H
#define _BCC_WASI_ERRNO_H
extern int errno;
#define errno errno
#define E2BIG 1
#define EACCES 2
#define EBADF 8
#define EDOM 18
#define EEXIST 20
#define EINVAL 28
#define EIO 29
#define EISDIR 31
#define ENOENT 44
#define ENOMEM 48
#define ENOSPC 51
#define ENOSYS 52
#define ENOTDIR 54
#define EPERM 63
#define ERANGE 68
#define ESPIPE 70
#define EILSEQ 25
#endif
