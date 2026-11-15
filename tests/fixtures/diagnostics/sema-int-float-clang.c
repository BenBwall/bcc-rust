// Clang's -Wimplicit-const-int-float-conversion: an integer constant that a
// floating type cannot hold exactly.
#define BIG 16777217
float a = 16777217;
double b = 9007199254740993LL;
float c = BIG;
float d = -16777217;
double e = 100000000000000001LL;
float f = (unsigned __int128)-1;
float g = ((unsigned __int128)1 << 127) + 1;
float exact = 16777216;
_Complex float complex_target = 16777217;
float cast = (float)16777217;
void take(float);
float x;
double y;
float uses(int i) {
    take(16777217);
    x = 16777217;
    x += 16777217;
    if (y > 9223372036854775807LL)
        return x * 16777217;
    return i ? 16777217 : x;
}
// Only the selected association of a generic selection is checked.
float unselected = _Generic(0, int: 1, default: 16777217 + 0.0f);
float unselected_literal = _Generic(0, int: 1, default: (float){16777217});
float selected = _Generic(0, int: 16777217 + 0.0f, default: 1);
float nested = _Generic(0, int: _Generic(0.0, double: 16777217 + 0.0f, default: 0), default: 1);
float unselected_nested = _Generic(0, int: _Generic(0.0, int: 16777217 + 0.0f, default: 0));
float in_function(void) {
    return _Generic(0, long: 16777217 + 0.0f, default: 16777219 + 0.0f);
}
// An exact significand whose exponent the type cannot hold.
_Float16 half = 65536;
_Float16 half_rounded = 65520;
_Float16 half_exact = 2048;
// Builtin calls that fold are constants.
float expected(void) { return __builtin_expect(16777217, 1); }
float swapped = __builtin_bswap32(0x01000001) + 0.0f;
float expected_sum(void) { return __builtin_expect(16777216, 1) + 1 + 0.0f; }
// APFloat truncates to about the digits it prints before rounding.
float truncated = ((unsigned __int128)52773793117994ULL << 64) | 5216560818296212498ULL;
