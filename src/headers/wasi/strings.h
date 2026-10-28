/* bcc's bundled C library for wasm32-wasip1: <strings.h>. */
#ifndef _BCC_WASI_STRINGS_H
#define _BCC_WASI_STRINGS_H
#include <stddef.h>
int strcasecmp(const char *s1, const char *s2);
int strncasecmp(const char *s1, const char *s2, size_t n);
#endif
