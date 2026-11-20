/* folded_nans.c with the floating-point environment accessed throughout,
 * which Clang implements as -ftrapping-math does: an invalid operation in a
 * function is computed at run time, unless it initializes a constant
 * automatic array or structure, which Clang's evaluator folds. GCC ignores
 * the pragma. C99 §7.6.1. */
#pragma STDC FENV_ACCESS ON
#include "folded_nans.c"
