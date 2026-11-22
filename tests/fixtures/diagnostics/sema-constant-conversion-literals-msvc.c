/* The MSVC identity reports cl's C4305 for a constant its conversion
   truncates, in place of Clang's constant conversions: none for a value
   that fits the target's width unsigned, whether written in hexadecimal or
   pasted, and one in code that does not run. */
#define CAT(a, b) a##b
char hexadecimal = 0xdf;
char pasted_decimal = CAT(2, 00);
char braced[2] = {223, 0};
void f(char *p) {
  *p = 0xdf;
  *p = 223;
  *p = 300;
  if (0)
    *p = 300;
  return;
  *p = 400;
}
