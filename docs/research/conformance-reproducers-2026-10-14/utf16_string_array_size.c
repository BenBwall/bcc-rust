/* C11 §6.7.9p22: an array of unknown size initialized by a u"" or U"" string
   literal takes its size from the literal (L"" works). bcc: "object requires
   a complete type: `a`". Clang and GCC print "6 16". */
#include <stdio.h>
#include <uchar.h>
char16_t a[] = u"ab";
char32_t b[] = U"xyz";
int main(void) { printf("%d %d\n", (int)sizeof a, (int)sizeof b); return 0; }
