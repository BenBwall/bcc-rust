/* Clang's -Wchar-subscripts under -Wall: a plain char subscript, either
   operand, unless a constant makes it not negative. */
typedef char Character;
int table[300];
int f(char c, signed char s, unsigned char u, Character t, const char k, char *p) {
  int *q = table;
  return table[c] + table[s] + table[u] + table[t] + table[k] + c[table]
         + table[*p] + q[c] + q[(char)1] + q[(char)-1] + q[(char)200]
         + table[c + 1] + table['x'] + table[(char)(c)];
}
