/* Program startup and termination for wasm32-wasip1.
   C99: §5.1.2.2.2-§5.1.2.2.3, p. 12; §7.20.4, pp. 314-316. */
#include <stdlib.h>
#include "wasi.h"

int errno;

#define ATEXIT_MAX 64
static void (*handlers[ATEXIT_MAX])(void);
static int handler_count;

int atexit(void (*function)(void)) {
    if (handler_count == ATEXIT_MAX)
        return -1;
    handlers[handler_count++] = function;
    return 0;
}

_Noreturn void _Exit(int status) {
    __bcc_wasi_proc_exit(status);
}

_Noreturn void _exit(int status) {
    __bcc_wasi_proc_exit(status);
}

_Noreturn void exit(int status) {
    while (handler_count > 0)
        handlers[--handler_count]();
    __wasm_call_dtors();
    __bcc_flush_all();
    __bcc_wasi_proc_exit(status);
}

/* Like a native abort's SIGABRT status 134 (128 + 6), so test harnesses
   compare the same exit status. */
_Noreturn void abort(void) {
    __bcc_flush_all();
    __bcc_wasi_proc_exit(134);
}

void _start(void) {
    __wasm_call_ctors();
    size_t count = 0, size = 0;
    char **argv = 0;
    if (__bcc_wasi_args_sizes_get(&count, &size) == 0 && count > 0) {
        argv = malloc((count + 1) * sizeof *argv);
        char *buffer = malloc(size);
        if (argv && buffer && __bcc_wasi_args_get(argv, buffer) == 0)
            argv[count] = 0;
        else
            count = 0;
    }
    exit(__bcc_main((int)count, argv));
}

static char **environment;
static int environment_read;

char *getenv(const char *name) {
    if (!environment_read) {
        environment_read = 1;
        size_t count = 0, size = 0;
        if (__bcc_wasi_environ_sizes_get(&count, &size) == 0 && count > 0) {
            environment = malloc((count + 1) * sizeof *environment);
            char *buffer = malloc(size);
            if (environment && buffer && __bcc_wasi_environ_get(environment, buffer) == 0)
                environment[count] = 0;
            else
                environment = 0;
        }
    }
    if (!environment)
        return 0;
    for (char **entry = environment; *entry; entry++) {
        const char *a = name, *b = *entry;
        while (*a && *a == *b) {
            a++;
            b++;
        }
        if (!*a && *b == '=')
            return (char *)b + 1;
    }
    return 0;
}

int system(const char *command) {
    return command ? -1 : 0;
}

void __bcc_assert_fail(const char *expression, const char *file, int line, const char *function);
int fprintf(void *stream, const char *format, ...);
extern void *const stderr;

void __bcc_assert_fail(const char *expression, const char *file, int line, const char *function) {
    fprintf(stderr, "%s:%d: %s: Assertion `%s' failed.\n", file, line, function, expression);
    abort();
}
