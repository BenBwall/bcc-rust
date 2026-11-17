/* Clang's -Wdeprecated-non-prototype for the names of library builtins
   that are no builtins: one -fno-builtin- names, and a GNU one in ISO C.
   Their declarations are reported and noted as any other; the reserved
   names and the other library functions stay builtins. */
long labs();
long labs(long);
long labs(long);
long labs();
long long llabs();
long long llabs(long long);
void *alloca();
void *alloca(unsigned long);
char *index();
char *index(const char *, int);
void *__builtin_alloca();
void *__builtin_alloca(unsigned long);
