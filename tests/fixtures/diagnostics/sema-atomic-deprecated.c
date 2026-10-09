#include <stdatomic.h>
atomic_int deprecated = ATOMIC_VAR_INIT(1);
#define W(x) ATOMIC_VAR_INIT(x)
atomic_int wrapped = W(2);
#define ATOMIC_VAR_INIT(value) (value)
atomic_int redefined = ATOMIC_VAR_INIT(3);
#ifdef ATOMIC_VAR_INIT
#endif
#ifndef ATOMIC_VAR_INIT
#endif
#if defined(ATOMIC_VAR_INIT)
#endif
#if defined ATOMIC_VAR_INIT
#endif
#define OLD 1
#pragma clang deprecated(OLD, "use " "NEW")
#define WRAP OLD
int legacy = WRAP;
#undef OLD
#define OLD 2
int replacement = OLD;
#pragma clang deprecated(__CLANG_ATOMIC_INT_LOCK_FREE)
int lock_free = ATOMIC_INT_LOCK_FREE;
_Pragma("clang deprecated(OLD, \"operator\")")
int operator_use = OLD;
int following;
