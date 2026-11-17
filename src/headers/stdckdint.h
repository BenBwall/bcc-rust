/* C23: 7.20, pp. 313-314; PDF pp. 326-327 (N3220). Checked integer
   arithmetic. GCC 14 and Clang ship this header in every language mode;
   Clang's defers to a hosted C library's header when there is one, as this
   one does. Each macro is its type-generic overflow builtin, which stores
   the wrapped result in *R and yields whether it overflowed. GCC's header
   casts the builtin's result to _Bool; the builtin's result is already
   bool, so the value and type agree, and leaving the cast out keeps
   `ckd_add(&r, a, b);` a statement with an effect in every mode. Under
   Clang's rules a call these macros write takes only C23's types, as in
   Clang, which finds the macro by its name (builtins/overflow.rs). The MSVC
   and TCC identities find no such header (headers::provides). */
#ifndef __BCC_STDCKDINT_H
#define __BCC_STDCKDINT_H
#if __STDC_HOSTED__ && __has_include_next(<stdckdint.h>)
#include_next <stdckdint.h>
#else
#define __STDC_VERSION_STDCKDINT_H__ 202311L
#define ckd_add(R, A, B) __builtin_add_overflow((A), (B), (R))
#define ckd_sub(R, A, B) __builtin_sub_overflow((A), (B), (R))
#define ckd_mul(R, A, B) __builtin_mul_overflow((A), (B), (R))
#endif
#endif
