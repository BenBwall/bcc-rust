/* sscanf edge cases whose results, stores and %n counts must match glibc's
   C99 sscanf: matching against input failures, suppressed conversions,
   field widths, scansets, "C" locale white space, and floating input
   including partial forms, infinities, NaNs and hexadecimal. glibc 2.39
   through 2.43 agree on these; scanf_cases_modern_glibc.c holds the cases
   that those releases changed. */
#include "scanf_show.h"
int main(void) {
  const char *cases[][2] = {
    {"+", "%d"}, {"1", "%*d%d"}, {"", "%d"}, {"", " "}, {"", "x"}, {" ", "%d"}, {"x", "%d"},
    {"1 2", "%d%n%d"}, {"12", "%1d%n%d"}, {"-", "%d%n"},
    {"08", "%i%n%d"}, {"1x", "%d%n x"}, {"1", "%d %n"}, {"1", "%d%n%d"}, {"ab", "%s%n%s"}, {"\v7", "%d"},
    {"a\vb", "%s%n%s"}, {"z-a", "%[z-a]%n"}, {"-", "%[a-]%n"}, {"b", "%[^a-c]%n"}, {"]x", "%[]x]%n"}, {"abc", "%2[abc]%n%s"},
    {"", "%[a]"}, {"x", "%[a]%n"}, {"ab", "%c%n%c"}, {"a", "%c%c"}, {"", "%n%d"}, {"5", "%d%%"}, {"5%", "%d%%%n"},
    {"nanabc", "%lf%n"}, {"infinity!", "%lf%n"}, {"infinit", "%lf%n"}, {"inx", "%lf%n"}, {"-inf", "%lf%n"},
    {"1.5e3x", "%lf%n"}, {".5", "%lf%n"}, {".", "%lf%n"}, {"1.", "%lf%n"}, {"+.e1", "%lf%n"},
    {"2.5 3.5", "%f%n%f"}, {"1e999", "%lf%n"}, {"0x1p3", "%lf%n"}, {"i", "%lf%n"}, {"", "%lf"}, {"z", "%lf"},
    {"  42", " %n%d"}, {"7", "%*d%n"}, {"1 2", "%d %*d %n"},
    {"\v\f\r\t 9", "%d"}, {"x\vy", "%s%n"}, {"5\v", "%d\v%n"}, {"a-z", "%[a-z-]%n"}, {"ba", "%[b-a]%n"},
    {"0x1.8p1z", "%lf%n"}, {"0X.8P-1", "%f%n"}, {"0x", "%lf%n"}, {"0xp1", "%lf%n"},
    {"-0x1p-1074", "%lf%n"}, {"0x1.000001p0", "%f%n"}, {"0x1.0000008p0", "%f%n"}, {"0x1.0000018p0", "%f%n"},
    {"1.00000017881393432617187499", "%f%n"}, {"3.4028235677973366e38", "%f%n"}, {"INFINITY", "%f%n"},
    {"-nan", "%lf%n"}, {"12345", "%3d%n%d"}, {"-12", "%2d%n%d"}, {"2.5 3.5", "%lf%n%d"},
    {"077 0x1A 10", "%i %i %i"}, {"-077", "%o%n"}, {"ff", "%x%n"}, {"4294967296", "%u%n"}, {"%", "%%%n"},
    {"  %", "%%%n"}, {"x", " x%n"}, {"xy", "x%ny"},
    {"0x0.0p-9223372036854775808", "%lf%n"}, {"0x1p99999999999999999999", "%lf%n"},
    {"0x1p-99999999999999999999", "%lf%n"}, {"-0x8p9223372036854775807", "%lf%n"},
    {"abcd-", "%[a-b-d]%n"}, {"-x-", "%[--x]%n"}, {"]^a", "%[]^a]%n"}, {"c-a", "%[a-c-a]%n"},
  };
  for (unsigned i = 0; i < sizeof cases / sizeof cases[0]; i++) show(cases[i][0], cases[i][1]);
  return 0;
}
