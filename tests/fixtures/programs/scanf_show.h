/* show() runs one sscanf case and prints its result, every store and the
   %n count, for the scanf_cases*.c fixtures. */
int printf(const char *, ...);
int sscanf(const char *, const char *, ...) __asm__("__isoc99_sscanf");
char *strstr(const char *, const char *);
void *memset(void *, int, unsigned long);
void *memcpy(void *, const void *, unsigned long);
static void show(const char *in, const char *fmt) {
  int a = -7, b = -7, n = -7; char s1[16], s2[16]; double d = -7; float f = -7, g = -7;
  memset(s1, 'Q', 16); memset(s2, 'Q', 16); s1[15] = s2[15] = 0;
  int r;
  /* Each pointer has the type its conversion stores. */
  if (strstr(fmt, "lf")) r = sscanf(in, fmt, &d, &n, &a);
  else if (strstr(fmt, "f")) r = sscanf(in, fmt, &f, &n, &g);
  else if (strstr(fmt, "[") || strstr(fmt, "s") || strstr(fmt, "c")) r = sscanf(in, fmt, s1, &n, s2);
  else r = sscanf(in, fmt, &a, &n, &b);
  unsigned long long bits;
  memcpy(&bits, &d, sizeof bits);
  printf("[%s] [%s] -> %d a=%d b=%d n=%d d=%a/%llx f=%a g=%a s1=%.15s s2=%.15s\n", in, fmt, r, a, b,
         n, d, bits, (double)f, (double)g, s1, s2);
}
