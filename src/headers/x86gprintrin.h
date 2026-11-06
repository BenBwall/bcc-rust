/* bcc umbrella of the x86 intrinsics that use general registers alone. */
#ifndef __X86GPRINTRIN_H
#define __X86GPRINTRIN_H
/* GCC's umbrella includes this, and programs rely on `ptrdiff_t` from it. */
#if defined(__GNUC__) && !defined(__clang__)
#include <stddef.h>
#endif
#include <popcntintrin.h>
#include <uintrintrin.h>
#endif
