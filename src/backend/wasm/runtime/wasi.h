/* The WASI preview 1 functions the runtime imports. The WebAssembly back
   end's linker turns each declared __bcc_wasi_NAME into the import NAME of
   the module wasi_snapshot_preview1. Every function returns a WASI errno
   (0 for success). */
#ifndef _BCC_WASI_H
#define _BCC_WASI_H
#include <stddef.h>

struct __bcc_ciovec {
    const void *buf;
    size_t len;
};

struct __bcc_iovec {
    void *buf;
    size_t len;
};

int __bcc_wasi_fd_write(int fd, const struct __bcc_ciovec *iovs, size_t iovs_len, size_t *nwritten);
int __bcc_wasi_fd_read(int fd, const struct __bcc_iovec *iovs, size_t iovs_len, size_t *nread);
int __bcc_wasi_args_sizes_get(size_t *argc, size_t *argv_buf_size);
int __bcc_wasi_args_get(char **argv, char *argv_buf);
int __bcc_wasi_environ_sizes_get(size_t *count, size_t *buf_size);
int __bcc_wasi_environ_get(char **environ, char *environ_buf);
int __bcc_wasi_clock_time_get(int clock, unsigned long long precision, unsigned long long *time);
_Noreturn void __bcc_wasi_proc_exit(int code);

/* memory.size and memory.grow of memory 0, in 64 KiB pages; grow returns
   the old size, or (size_t) -1 on failure. */
size_t __bcc_wasm_memory_size(void);
size_t __bcc_wasm_memory_grow(size_t pages);

/* Linker-provided: the first address after the static data. */
extern unsigned char __heap_base;

/* Linker-provided: the program's main with (argc, argv), and the
   constructor and destructor runners. */
int __bcc_main(int argc, char **argv);
void __wasm_call_ctors(void);
void __wasm_call_dtors(void);

/* Writes every byte of buf to fd; 0, or a WASI errno. */
int __bcc_write_all(int fd, const void *buf, size_t len);
/* Flushes every stdio stream. */
void __bcc_flush_all(void);

#endif
