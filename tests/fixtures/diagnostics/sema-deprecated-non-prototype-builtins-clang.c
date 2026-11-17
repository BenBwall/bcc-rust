/* Clang's -Wdeprecated-non-prototype for declarations of builtins: it
   neither reports a declaration of a builtin without a prototype nor notes
   a prototype that declares one, and a later declaration takes the builtin
   from the one it redeclares only after Clang has compared them. */
long labs();
long labs(long);
long long llabs(long long);
long long llabs();
int __builtin_abs();
int __builtin_abs(int);
int __builtin_ffs(int);
int __builtin_ffs();
void *alloca();
void *alloca(unsigned long);
double sin(double);
double sin();
double sin(double);
static int abs();
int abs(int);
void scope(void) {
  double cos();
}
double cos(double);
double tan(double);
void later(void) {
  double tan();
}
int unrelated();
int unrelated(int);
