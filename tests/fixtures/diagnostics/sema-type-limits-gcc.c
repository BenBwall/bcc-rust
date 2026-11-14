int f(unsigned u, unsigned char c, signed char s, int i, unsigned long ul, _Bool b, unsigned short us) {
  int r = 0;
  r += u < 0;
  r += u >= 0;
  r += 0 > u;
  r += 0 <= u;
  r += u <= 0;
  r += u > 0;
  r += u == 0;
  r += c > 300;
  r += c < 300;
  r += c == 300;
  r += c != 300;
  r += 300 > c;
  r += c >= 0;
  r += c < 0;
  r += s > 200;
  r += s < -200;
  r += s == 128;
  r += c > 255;
  r += c >= 255;
  r += i < 0;
  r += ul < 0;
  r += c < -1;
  r += b > 1;
  r += b == 2;
  r += us > 70000;
  r += (c + 1) > 300;
  r += c > 300u;
  return r;
}
#define ZERO 0
#define BIG 300
enum { EZERO };
int g(signed char s, unsigned u, unsigned char c, int i, short h) {
  int r = 0;
  r += s == 4294967295U;
  r += s < 4294967295U;
  r += s < 200U;
  r += s == 200U;
  r += s != 200U;
  r += s <= 4294967295U;
  r += s >= 0U;
  r += 4294967295U == h;
  r += i < 0U;
  r += i <= 2147483647;
  r += i == 4294967296L;
  r += u <= 4294967295U;
  r += u == -1;
  r += u < ZERO;
  r += u >= (ZERO);
  r += u < EZERO;
  r += c > BIG;
  r += sizeof(c > 300);
  r += sizeof(u < 0);
  r += 0 ? (c > 300) : 1;
  r += 0 && (c > 300);
  r += _Generic(1, long: (c > 300), default: 1);
  return r;
}
int k(unsigned char c, unsigned u) { if (0) return c > 300; if (0) return u < 0; return 0; }
int q(unsigned char c) { return __builtin_constant_p(c > 300) + (0 ? (c <= 255) : 1); }
int r(unsigned __int128 u) { return u <= (((unsigned __int128)1 << 127) - 1); }
#define ID(x) x
#define false 0
int m(unsigned u, unsigned char c) {
  int r = 0;
  r += u < ID(0);
  r += u >= ID((0));
  r += c <= ID(255);
  r += c > ID(300);
  r += u < false;
  return r;
}
