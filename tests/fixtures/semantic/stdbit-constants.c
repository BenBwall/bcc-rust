/* The type-generic bit builtins fold in integer constant expressions at
   every width, under both identities (GCC 15 and Clang 23 accept all of
   these); their result types are int, unsigned int, bool or the
   operand's type. */
#define TYPE(e, T) _Generic((e), T: 1, default: 0)

_Static_assert(__builtin_popcountg(0xf0u) == 4);
_Static_assert(__builtin_popcountg((unsigned char)0xff) == 8);
_Static_assert(__builtin_popcountg((unsigned __int128)-1) == 128);
_Static_assert(__builtin_popcountg((unsigned _BitInt(1000))-1) == 1000);
_Static_assert(__builtin_clzg(1u) == 31);
_Static_assert(__builtin_clzg((unsigned char)1) == 7);
_Static_assert(__builtin_clzg((unsigned _BitInt(7))1) == 6);
_Static_assert(__builtin_clzg((unsigned __int128)1) == 127);
_Static_assert(__builtin_clzg((unsigned _BitInt(200))1 << 150) == 49);
_Static_assert(__builtin_clzg(0u, -5) == -5);
_Static_assert(__builtin_clzg((unsigned _BitInt(1000))0, (short)7) == 7);
_Static_assert(__builtin_ctzg(0x100ull) == 8);
_Static_assert(__builtin_ctzg((unsigned _BitInt(1000))1 << 999) == 999);
_Static_assert(__builtin_ctzg((unsigned _BitInt(1))0, 42) == 42);
_Static_assert(TYPE(__builtin_popcountg(1u), int) && TYPE(__builtin_clzg(1u, 2), int));

_Static_assert(__builtin_stdc_leading_zeros(0u) == 32);
_Static_assert(__builtin_stdc_leading_zeros((unsigned _BitInt(65))1) == 64);
_Static_assert(__builtin_stdc_leading_ones((unsigned short)0xff00) == 8);
_Static_assert(__builtin_stdc_leading_ones((unsigned _BitInt(1000))-1) == 1000);
_Static_assert(__builtin_stdc_trailing_zeros((unsigned char)0) == 8);
_Static_assert(__builtin_stdc_trailing_ones(0x7ull) == 3);
_Static_assert(__builtin_stdc_trailing_ones((unsigned _BitInt(200))-1 >> 1) == 199);
_Static_assert(__builtin_stdc_first_leading_zero((unsigned char)0xff) == 0);
_Static_assert(__builtin_stdc_first_leading_zero((unsigned char)0xf0) == 5);
_Static_assert(__builtin_stdc_first_leading_one(0u) == 0);
_Static_assert(__builtin_stdc_first_leading_one(1u) == 32);
_Static_assert(__builtin_stdc_first_trailing_zero(0x7u) == 4);
_Static_assert(__builtin_stdc_first_trailing_zero(~0u) == 0);
_Static_assert(__builtin_stdc_first_trailing_one(0x8u) == 4);
_Static_assert(__builtin_stdc_first_trailing_one((unsigned _BitInt(1000))1 << 700) == 701);
_Static_assert(__builtin_stdc_count_zeros((unsigned _BitInt(63))1) == 62);
_Static_assert(__builtin_stdc_count_ones((unsigned __int128)0x55) == 4);
_Static_assert(__builtin_stdc_has_single_bit(64u) && !__builtin_stdc_has_single_bit(65u));
_Static_assert(!__builtin_stdc_has_single_bit(0u));
_Static_assert(__builtin_stdc_bit_width(0u) == 0 && __builtin_stdc_bit_width(5u) == 3);
_Static_assert(__builtin_stdc_bit_width((unsigned _BitInt(1000))1 << 999) == 1000);
_Static_assert(__builtin_stdc_bit_floor(0u) == 0 && __builtin_stdc_bit_floor(100u) == 64);
_Static_assert(__builtin_stdc_bit_floor((unsigned _BitInt(300))-1) == (unsigned _BitInt(300))1 << 299);
_Static_assert(__builtin_stdc_bit_ceil(0u) == 1 && __builtin_stdc_bit_ceil(1u) == 1);
_Static_assert(__builtin_stdc_bit_ceil(5u) == 8 && __builtin_stdc_bit_ceil(64u) == 64);
_Static_assert(__builtin_stdc_bit_ceil(0x80000001u) == 0);
_Static_assert(__builtin_stdc_bit_ceil((unsigned char)200) == 0);
_Static_assert(__builtin_stdc_bit_ceil((unsigned _BitInt(1))0) == 1);
_Static_assert(__builtin_stdc_bit_ceil((unsigned _BitInt(300))3) == 4);
_Static_assert(__builtin_stdc_rotate_left(1u, 33) == 2);
_Static_assert(__builtin_stdc_rotate_left((unsigned char)0x81, 1) == 3);
_Static_assert(__builtin_stdc_rotate_right(1u, ((unsigned _BitInt(300))1 << 299) + 4) == 1u << 28);
_Static_assert(__builtin_stdc_rotate_left((unsigned _BitInt(200))1, 199) == (unsigned _BitInt(200))1 << 199);
_Static_assert(__builtin_stdc_rotate_left((unsigned _BitInt(1000))3, 999) == ((unsigned _BitInt(1000))1 << 999) + 1);
_Static_assert(TYPE(__builtin_stdc_leading_zeros((unsigned char)1), unsigned));
_Static_assert(TYPE(__builtin_stdc_has_single_bit(1u), bool));
_Static_assert(TYPE(__builtin_stdc_bit_floor((unsigned char)1), unsigned char));
_Static_assert(TYPE(__builtin_stdc_bit_ceil((unsigned _BitInt(7))1), unsigned _BitInt(7)));
_Static_assert(TYPE(__builtin_stdc_rotate_left((const unsigned long)1, 1), unsigned long));

/* GCC only: the signed builtins, parity, and clzg/ctzg of zero. */
#ifdef BCC_TEST_GCC
_Static_assert(__builtin_parityg(7u) == 1 && __builtin_parityg((unsigned _BitInt(1000))3) == 0);
_Static_assert(__builtin_ffsg(-8) == 4 && __builtin_ffsg(0) == 0);
_Static_assert(__builtin_ffsg((_BitInt(1000))1 << 900) == 901);
_Static_assert(__builtin_clrsbg(-1) == 31 && __builtin_clrsbg(0) == 31);
_Static_assert(__builtin_clrsbg((signed char)-3) == 5 && __builtin_clrsbg((_BitInt(200))5) == 196);
_Static_assert(__builtin_clrsbg((__int128)-1) == 127);
_Static_assert(__builtin_clzg(0u) == 32 && __builtin_ctzg((unsigned char)0) == 8);
_Static_assert(__builtin_clzg((unsigned _BitInt(1000))0) == 1000);
#endif

/* Clang only: bool and enumerated operands, and negative rotation
   counts. */
#ifdef BCC_TEST_CLANG
enum e { E5 = 5 };
_Static_assert(__builtin_clzg((bool)1) == 0 && __builtin_popcountg((enum e)E5) == 2);
_Static_assert(__builtin_clzg((enum e)E5) == 29);
_Static_assert(__builtin_stdc_rotate_left((bool)1, 3) == 1);
_Static_assert(TYPE(__builtin_stdc_rotate_left((bool)1, 3), bool));
_Static_assert(__builtin_stdc_rotate_left(1u, -1) == 0x80000000u);
_Static_assert(__builtin_stdc_rotate_left((unsigned _BitInt(7))1, -1) == 0x40);
_Static_assert(__builtin_stdc_rotate_right((unsigned char)1, -9) == 2);
_Static_assert(__builtin_stdc_rotate_right(1u, -(((_BitInt(300))1 << 290) + 1)) == 2);
/* GCC 15 does not fold a rotation of this width. */
_Static_assert(__builtin_stdc_rotate_right((unsigned _BitInt(7))1, 1) == 0x40);
#endif
int following;
