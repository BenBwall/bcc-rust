/* bcc x86 intrinsic resource umbrella. */
#ifndef __X86INTRIN_H
#define __X86INTRIN_H
#include <immintrin.h>
/* As Clang's umbrella does, take <mm3dnow.h>'s contents without its
   deprecation #warning, which a system header still emits. */
#include <mmintrin.h>
#include <prfchwintrin.h>
#endif
