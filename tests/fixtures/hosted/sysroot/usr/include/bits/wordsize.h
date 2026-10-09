/* A fake C library header that follows glibc's <bits/wordsize.h>. */
#if defined __x86_64__ && !defined __ILP32__
# define __WORDSIZE 64
#else
# define __WORDSIZE 32
#endif
