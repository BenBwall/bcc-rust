/* Translation unit 2 of 2. bcc: "multiple definition of strncasecmp/strcasecmp"
   at link time; Clang and GCC link and print "0 0". */
#include <stdio.h>
#include <string.h>
int one(void);
int main(void) { printf("%d %d\n", one(), strncasecmp("ab", "AB", 2)); return 0; }
