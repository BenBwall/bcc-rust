/* Typed GNU math builtins are patchy: __builtin_floor, __builtin_fabs, __builtin_inf, __builtin_isinf,
   __builtin_copysign and __builtin_signbit compile; __builtin_ceil, __builtin_sqrt, __builtin_sin, __builtin_pow,
   __builtin_exp, __builtin_log, __builtin_round, __builtin_trunc, __builtin_fma, __builtin_fmin, __builtin_fabsf,
   __builtin_inff, __builtin_isnan, __builtin_rotateleft32 and the __atomic_* / __sync_* family are refused with
   "construct not supported by code generation: its type was not analyzed" (SQLite's shell.c on glibc). */
double f(double r) { return __builtin_ceil(r); }
int main(void) { return f(1.5) == 2.0 ? 0 : 1; }
