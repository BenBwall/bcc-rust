/*===------------------ uintrintrin.h - UINTR intrinsics -------------------===
 *
 * Part of the LLVM Project, under the Apache License v2.0 with LLVM Exceptions.
 * See https://llvm.org/LICENSE.txt for license information.
 * SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
 *
 *===-----------------------------------------------------------------------===
 */

/* The instructions are written as assembly, since bcc has no builtins for
   them; both compilers' forms are available whatever -muintr says, as GCC's
   header makes them. */

#ifndef __X86GPRINTRIN_H
#error "Never use <uintrintrin.h> directly; include <x86gprintrin.h> instead."
#endif

#ifndef __UINTRINTRIN_H
#define __UINTRINTRIN_H

#ifdef __x86_64__

/* What the processor pushes for a user interrupt, which a handler's first
   parameter points at; its second parameter is the vector. */
struct __uintr_frame
{
  unsigned long long rip;
  unsigned long long rflags;
  unsigned long long rsp;
};

static __inline__ void __attribute__((__always_inline__))
_clui (void)
{
  __asm__ __volatile__ ("clui");
}

static __inline__ void __attribute__((__always_inline__))
_stui (void)
{
  __asm__ __volatile__ ("stui");
}

static __inline__ unsigned char __attribute__((__always_inline__))
_testui (void)
{
  unsigned char __result;
  __asm__ __volatile__ ("testui\n\tsetc %0" : "=q" (__result) : : "cc");
  return __result;
}

static __inline__ void __attribute__((__always_inline__))
_senduipi (unsigned long long __index)
{
  __asm__ __volatile__ ("senduipi %0" : : "r" (__index));
}

#endif /* __x86_64__ */

#endif /* __UINTRINTRIN_H */
