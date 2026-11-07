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
