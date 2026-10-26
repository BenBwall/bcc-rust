/* sscanf edge cases whose results, stores and %n counts must match glibc's
   C99 sscanf: matching against input failures, suppressed conversions,
   field widths, scansets, "C" locale white space, and floating input
   including partial forms, infinities, NaNs and hexadecimal. */
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
int main(void) {
  const char *cases[][2] = {
    {"+", "%d"}, {"1", "%*d%d"}, {"1", "%2c"}, {"", "%d"}, {"", " "}, {"", "x"}, {" ", "%d"}, {"x", "%d"},
    {"1 2", "%d%n%d"}, {"12", "%1d%n%d"}, {"-", "%d%n"}, {"0x", "%x%n"}, {"0x", "%i%n"}, {"0xg", "%i%n"},
    {"08", "%i%n%d"}, {"1x", "%d%n x"}, {"1", "%d %n"}, {"1", "%d%n%d"}, {"ab", "%s%n%s"}, {"\v7", "%d"},
    {"a\vb", "%s%n%s"}, {"z-a", "%[z-a]%n"}, {"-", "%[a-]%n"}, {"b", "%[^a-c]%n"}, {"]x", "%[]x]%n"}, {"abc", "%2[abc]%n%s"},
    {"", "%[a]"}, {"x", "%[a]%n"}, {"ab", "%c%n%c"}, {"a", "%c%c"}, {"", "%n%d"}, {"5", "%d%%"}, {"5%", "%d%%%n"},
    {"nanabc", "%lf%n"}, {"infinity!", "%lf%n"}, {"infinit", "%lf%n"}, {"inx", "%lf%n"}, {"-inf", "%lf%n"}, {"NaN(12)z", "%lf%n"},
    {"1e", "%lf%n"}, {"1e+", "%lf%n"}, {"1.5e3x", "%lf%n"}, {".5", "%lf%n"}, {".", "%lf%n"}, {"1.", "%lf%n"}, {"+.e1", "%lf%n"},
    {"2.5 3.5", "%f%n%f"}, {"1e999", "%lf%n"}, {"0x1p3", "%lf%n"}, {"nan(", "%lf%n"}, {"i", "%lf%n"}, {"", "%lf"}, {"z", "%lf"},
    {"  42", " %n%d"}, {"7", "%*d%n"}, {"1 2", "%d %*d %n"},
    {"\v\f\r\t 9", "%d"}, {"x\vy", "%s%n"}, {"5\v", "%d\v%n"}, {"a-z", "%[a-z-]%n"}, {"ba", "%[b-a]%n"},
    {"0x1.8p1z", "%lf%n"}, {"0X.8P-1", "%f%n"}, {"0x", "%lf%n"}, {"0xp1", "%lf%n"}, {"0x1p", "%lf%n"},
    {"-0x1p-1074", "%lf%n"}, {"0x1.000001p0", "%f%n"}, {"0x1.0000008p0", "%f%n"}, {"0x1.0000018p0", "%f%n"},
    {"1.00000017881393432617187499", "%f%n"}, {"3.4028235677973366e38", "%f%n"}, {"INFINITY", "%f%n"},
    {"nan(0x10)", "%lf%n"}, {"-nan", "%lf%n"}, {"12345", "%3d%n%d"}, {"-12", "%2d%n%d"}, {"1e5", "%2lf%n%d"}, {"2.5 3.5", "%lf%n%d"},
    {"077 0x1A 10", "%i %i %i"}, {"-077", "%o%n"}, {"ff", "%x%n"}, {"4294967296", "%u%n"}, {"%", "%%%n"},
    {"  %", "%%%n"}, {"x", " x%n"}, {"xy", "x%ny"},
    {"0x0.0p-9223372036854775808", "%lf%n"}, {"0x1p99999999999999999999", "%lf%n"},
    {"0x1p-99999999999999999999", "%lf%n"}, {"-0x8p9223372036854775807", "%lf%n"},
    {"abcd-", "%[a-b-d]%n"}, {"-x-", "%[--x]%n"}, {"]^a", "%[]^a]%n"}, {"c-a", "%[a-c-a]%n"},
    {"nan(18446744073709551616)", "%lf%n"}, {"nan(0x10000000000000000)", "%lf%n"},
    {"nan(077)", "%lf%n"}, {"nan(08)", "%lf%n"}, {"nan(0x)", "%lf%n"}, {"nan()", "%lf%n"},
    {"-nan(1234567)", "%f%n"},
  };
  for (unsigned i = 0; i < sizeof cases / sizeof cases[0]; i++) show(cases[i][0], cases[i][1]);
  return 0;
}
