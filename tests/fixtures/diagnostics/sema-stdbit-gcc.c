/* Misused type-generic bit builtins under GCC's rules: GCC refuses signed,
   bool and enumerated operands (ffsg and clrsbg take signed ones), a
   result for zero that is not an int after GCC's promotions, and an
   enumerated rotation count. */
enum e { A, B };
struct s { int i; } object;
int signed_operand(int x) { return __builtin_popcountg(x); }
int bool_operand(bool x) { return __builtin_clzg(x); }
int enum_operand(enum e x) { return __builtin_stdc_leading_zeros(x); }
int float_operand(double x) { return __builtin_ctzg(x); }
int pointer_operand(int *x) { return __builtin_stdc_bit_floor(x); }
int structure_operand(void) { return __builtin_popcountg(object); }
int unsigned_ffs(unsigned x) { return __builtin_ffsg(x); }
int unsigned_clrsb(unsigned x) { return __builtin_clrsbg(x); }
int bool_ffs(bool x) { return __builtin_ffsg(x); }
int signed_parity(long x) { return __builtin_parityg(x); }
int signed_rotation(int x) { return __builtin_stdc_rotate_left(x, 1); }
int long_fallback(unsigned x) { return __builtin_clzg(x, 1L); }
int unsigned_fallback(unsigned x) { return __builtin_ctzg(x, 1u); }
int floating_fallback(unsigned x) { return __builtin_clzg(x, 1.0); }
int wide_fallback(unsigned x) { return __builtin_clzg(x, (unsigned _BitInt(32))1); }
int enum_fallback(unsigned x) { return __builtin_clzg(x, (enum e)1); }
int narrow_fallback(unsigned x) { return __builtin_clzg(x, (unsigned _BitInt(31))1) + __builtin_ctzg(x, (short)1); }
int enum_count(unsigned x) { return __builtin_stdc_rotate_left(x, (enum e)1); }
int float_count(unsigned x) { return __builtin_stdc_rotate_right(x, 1.5); }
int bool_count(unsigned x) { return __builtin_stdc_rotate_right(x, (bool)1); }
int missing(void) { return __builtin_popcountg(); }
int extra(unsigned x) { return __builtin_popcountg(x, 1) + __builtin_clzg(x, 1, 2); }
int missing_count(unsigned x) { return __builtin_stdc_rotate_left(x); }
int following;
