/* Which integer type an enumeration is compatible with (C99 6.7.2.2p4):
   GCC documents unsigned int with no negative constant, int otherwise. */
#include <stdio.h>
enum plain { A, B };
enum negative { M = -1, N };
int main(void) {
    printf("plain=unsigned:%d int:%d negative=unsigned:%d int:%d\n",
           __builtin_types_compatible_p(enum plain, unsigned int),
           __builtin_types_compatible_p(enum plain, int),
           __builtin_types_compatible_p(enum negative, unsigned int),
           __builtin_types_compatible_p(enum negative, int));
    return 0;
}
