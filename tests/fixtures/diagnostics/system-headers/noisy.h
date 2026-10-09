/* Found through -isystem, so a system header: its warnings and extension
   diagnostics are withheld, even under -pedantic-errors; errors remain. */
#ifndef NOISY_H
#define NOISY_H
#if UNDEFINED_IN_SYSTEM_HEADER
#endif
#define __INT64_C(c) c ## L
#define NOISY_VALUE 1
#define NOISY_WIDE long long
long long system_long_long;
#include "beside-noisy.h"
#error a system header still reports errors
#endif
