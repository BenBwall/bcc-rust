/* Clang's -Wdeprecated-non-prototype when -ffreestanding makes no library
   function a builtin: the declarations of one are reported and noted as
   any other, and the reserved names stay builtins. */
long labs();
long labs(long);
double sin(double);
double sin();
int __builtin_abs();
int __builtin_abs(int);
double __builtin_sin(double);
double __builtin_sin();
