/*===---- xsaveintrin.h - XSAVE intrinsic ----------------------------------===
 *
 * Part of the LLVM Project, under the Apache License v2.0 with LLVM Exceptions.
 * See https://llvm.org/LICENSE.txt for license information.
 * SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
 *
 *===-----------------------------------------------------------------------===
 */

/* bcc implements `_xgetbv` of this header, in each compiler's form: Clang's
   macro, so that a caller without XSAVE hears that the builtin needs it, and
   GCC's `always_inline` function with XSAVE, whose result is `long long`,
   so that such a caller hears that inlining failed. GCC declares it in
   <x86gprintrin.h>, which its <immintrin.h> includes; Clang in
   <immintrin.h>. */

#if !defined(__IMMINTRIN_H) && !defined(__X86GPRINTRIN_H)
#error "Never use <xsaveintrin.h> directly; include <immintrin.h> instead."
#endif

#ifndef __XSAVEINTRIN_H
#define __XSAVEINTRIN_H

#ifdef __clang__
#define _xgetbv(A) __builtin_ia32_xgetbv((long long)(A))
#else
static __inline__ long long __attribute__((__always_inline__, __nodebug__, __target__("xsave")))
_xgetbv(unsigned int __A) {
  return __builtin_ia32_xgetbv(__A);
}
#endif

#endif /* __XSAVEINTRIN_H */
