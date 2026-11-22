/* cl's C4244, C4305 and C4245 at its level 4, as `cl /W4` writes them. */
typedef unsigned char Byte;
typedef unsigned int U32;
struct S { Byte b; unsigned short w; };
void take(Byte b, float f);
Byte global = 300;
float half = 0.5, tenth = 0.1;

static Byte low(U32 value)
{
    return value;
}

int conversions(int i, U32 u, long long ll, double d, const int c,
                struct S *s, const struct S *k)
{
    char ch = i;
    short narrow = 70000;
    unsigned positive = -1;
    float f = d;
    s->b = i;
    s->w = u >> 16;
    s->b = u & 0xff;
    ch = c;
    ch = ch + 1;
    ch = i % 10;
    ch += i;
    ch |= i;
    f = i;
    f = (short)i;
    i = ll;
    i = d * 2;
    take(i, d);
    take(k->b, 1.0);
    i = k->w + ll;
    if (2)
        return low(u) + narrow + positive;
    while (1) {
        if (sizeof(int) == 4)
            break;
    }
    do { ch++; } while (0);
    return ch + (int)f;
}
