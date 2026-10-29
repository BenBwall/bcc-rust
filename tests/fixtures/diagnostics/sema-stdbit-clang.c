/* Misused type-generic bit builtins under Clang's rules: Clang refuses a
   signed operand, takes bool and enumerations except in the
   __builtin_stdc_* forms other than the rotations, requires an int result
   for zero after the integer promotions (which leave _BitInt alone), and
   takes a rotation count of any integer type. */
enum e { A, B };
enum s { Negative = -1 };
int signed_operand(int x) { return __builtin_popcountg(x); }
int signed_enum_operand(enum s x) { return __builtin_clzg(x); }
int bool_operand(bool x) { return __builtin_clzg(x) + __builtin_stdc_rotate_left(x, 1); }
int enum_operand(enum e x) { return __builtin_popcountg(x) + __builtin_stdc_rotate_right(x, 1); }
int stdc_bool_operand(bool x) { return __builtin_stdc_leading_zeros(x); }
int stdc_enum_operand(enum e x) { return __builtin_stdc_bit_width(x); }
int float_operand(float x) { return __builtin_stdc_count_ones(x); }
int unsigned_fallback(unsigned x) { return __builtin_clzg(x, 1u); }
int bit_int_fallback(unsigned x) { return __builtin_ctzg(x, (_BitInt(32))1); }
int enum_fallback(unsigned x) { return __builtin_clzg(x, (enum e)1) + __builtin_clzg(x, (enum s)1); }
int promoted_fallback(unsigned x) { return __builtin_clzg(x, (char)1) + __builtin_ctzg(x, (bool)1); }
int pointer_count(unsigned x) { return __builtin_stdc_rotate_left(x, (int *)0); }
int enum_count(unsigned x) { return __builtin_stdc_rotate_left(x, (enum e)1); }
int too_many(unsigned x) { return __builtin_stdc_bit_ceil(x, 1); }
enum fixed : unsigned char { Fixed };
int fixed_operand(enum fixed x) { return __builtin_popcountg(x) + __builtin_stdc_leading_ones(x); }
int following;
