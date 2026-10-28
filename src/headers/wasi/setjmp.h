/* bcc's bundled C library for wasm32-wasip1: <setjmp.h>. WebAssembly
   without the exception-handling proposal cannot return to a frame twice,
   so the WebAssembly back end refuses calls of these functions. */
#ifndef _BCC_WASI_SETJMP_H
#define _BCC_WASI_SETJMP_H
typedef long long jmp_buf[8];
int setjmp(jmp_buf env) __attribute__((returns_twice));
_Noreturn void longjmp(jmp_buf env, int val);
#endif
