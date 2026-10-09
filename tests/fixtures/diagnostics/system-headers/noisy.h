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
#define NOISY_PROLOG(name) (!defined(NOISY_DEFINED_ ## name))
#define NOISY_DEFINED defined(NOISY_H)
#if NOISY_PROLOG(x) && NOISY_DEFINED
#endif
#error a system header still reports errors
#endif
