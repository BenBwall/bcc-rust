/* sscanf cases on which glibc 2.43 (whose C99 behaviour the interpreter
   follows) differs from earlier releases; scanf_cases_modern_glibc.expected
   records its output. Each group names the glibc commit that changed it. */
#include "scanf_show.h"
int main(void) {
  const char *cases[][2] = {
    /* 2.41, BZ #30647 (04e8698fcc): "nan(n-char-sequence)" is consumed and
       sets the payload; before, matching stopped after "nan". */
    {"NaN(12)z", "%lf%n"}, {"nan(", "%lf%n"}, {"nan(0x10)", "%lf%n"},
    {"nan(18446744073709551616)", "%lf%n"}, {"nan(0x10000000000000000)", "%lf%n"},
    {"nan(077)", "%lf%n"}, {"nan(08)", "%lf%n"}, {"nan(0x)", "%lf%n"}, {"nan()", "%lf%n"},
    {"-nan(1234567)", "%f%n"},
    /* 2.42, BZ #12701: an integer prefix without digits (e78cb4f6bd) and an
       exponent without digits (0a8e7ac95c) are matching failures; before,
       they matched as zero or as the significand alone (also when a field
       width cuts the exponent's digits off). */
    {"0x", "%x%n"}, {"0x", "%i%n"}, {"0xg", "%i%n"}, {"1e", "%lf%n"}, {"1e+", "%lf%n"},
    {"0x1p", "%lf%n"}, {"1e5", "%2lf%n%d"},
    /* 2.43: %Nc fails without N characters (BZ #12701, 2b16c76609). */
    {"1", "%2c"},
  };
  for (unsigned i = 0; i < sizeof cases / sizeof cases[0]; i++) show(cases[i][0], cases[i][1]);
  return 0;
}
