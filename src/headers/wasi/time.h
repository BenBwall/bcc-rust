/* bcc's bundled C library for wasm32-wasip1: <time.h>. */
#ifndef _BCC_WASI_TIME_H
#define _BCC_WASI_TIME_H
#include <stddef.h>
typedef long long time_t;
typedef long long clock_t;
#define CLOCKS_PER_SEC 1000000LL
struct timespec { time_t tv_sec; long tv_nsec; };
clock_t clock(void);
time_t time(time_t *timer);
double difftime(time_t time1, time_t time0);
#endif
