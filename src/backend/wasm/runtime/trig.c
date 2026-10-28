/* Trigonometric functions for wasm32-wasip1: sin, cos and tan with
   correctly reduced arguments for every finite input, and their float
   variants.

   Translated to C from rust-lang/libm 0.2.16 (MIT), whose routines are
   musl's, which come from FreeBSD's msun: e_rem_pio2.c (rem_pio2),
   k_rem_pio2.c (rem_pio2_large, Payne-Hanek reduction), k_sin.c, k_cos.c,
   k_tan.c, s_sin.c, s_cos.c and s_tan.c.

   rust-lang/libm: Copyright (c) 2018 Jorge Aparicio. musl libc: Copyright
   (c) 2005-2020 Rich Felker, et al. Under the MIT license:

   Permission is hereby granted, free of charge, to any person obtaining a
   copy of this software and associated documentation files (the
   "Software"), to deal in the Software without restriction, including
   without limitation the rights to use, copy, modify, merge, publish,
   distribute, sublicense, and/or sell copies of the Software, and to
   permit persons to whom the Software is furnished to do so, subject to
   the following conditions:

   The above copyright notice and this permission notice shall be included
   in all copies or substantial portions of the Software.

   THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS
   OR IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF
   MERCHANTABILITY, FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT.
   IN NO EVENT SHALL THE AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY
   CLAIM, DAMAGES OR OTHER LIABILITY, WHETHER IN AN ACTION OF CONTRACT,
   TORT OR OTHERWISE, ARISING FROM, OUT OF OR IN CONNECTION WITH THE
   SOFTWARE OR THE USE OR OTHER DEALINGS IN THE SOFTWARE.

   The FreeBSD and fdlibm routines carry Sun's notice:

   ====================================================
   Copyright (C) 1993 by Sun Microsystems, Inc. All rights reserved.

   Developed at SunPro, a Sun Microsystems, Inc. business.
   Permission to use, copy, modify, and distribute this
   software is freely granted, provided that this notice
   is preserved.
   ==================================================== */
#include <math.h>
#include <stdint.h>

static uint64_t trig_bits(double x) {
    union {
        double d;
        uint64_t u;
    } v = {x};
    return v.u;
}

static double trig_from_bits(uint64_t u) {
    union {
        uint64_t u;
        double d;
    } v = {u};
    return v.d;
}

/* 2/pi in 24-bit chunks: ipio2[i] * 2^(-24(i+1)). Enough for every double. */
static const int32_t IPIO2[66] = {
    0xA2F983, 0x6E4E44, 0x1529FC, 0x2757D1, 0xF534DD, 0xC0DB62,
    0x95993C, 0x439041, 0xFE5163, 0xABDEBB, 0xC561B7, 0x246E3A,
    0x424DD2, 0xE00649, 0x2EEA09, 0xD1921C, 0xFE1DEB, 0x1CB129,
    0xA73EE8, 0x8235F5, 0x2EBB44, 0x84E99C, 0x7026B4, 0x5F7E41,
    0x3991D6, 0x398353, 0x39F49C, 0x845F8B, 0xBDF928, 0x3B1FF8,
    0x97FFDE, 0x05980F, 0xEF2F11, 0x8B5A0A, 0x6D1F6D, 0x367ECF,
    0x27CB09, 0xB74F46, 0x3F669E, 0x5FEA2D, 0x7527BA, 0xC7EBE5,
    0xF17B3D, 0x0739F7, 0x8A5292, 0xEA6BFB, 0x5FB11F, 0x8D5D08,
    0x560330, 0x46FC7B, 0x6BABF0, 0xCFBC20, 0x9AF436, 0x1DA9E3,
    0x91615E, 0xE61B08, 0x659985, 0x5F14A0, 0x68408D, 0xFFD880,
    0x4D7327, 0x310606, 0x1556CA, 0x73A8C9, 0x60E27B, 0xC08C6B,
};

static const double PIO2[8] = {
    1.57079625129699707031e+00, 7.54978941586159635335e-08, 5.39030252995776476554e-15,
    3.28200341580791294123e-22, 1.27065575308067607349e-29, 1.22933308981111328932e-36,
    2.73370053816464559624e-44, 2.16741683877804819444e-51,
};

/* Payne-Hanek reduction of x = x[0..nx) * 2^e0 (24-bit chunks) by pi/2
   into y[0] + y[1], returning the quadrant (prec 1: double precision). */
static int rem_pio2_large(const double *x, double *y, int e0, int nx) {
    const double x1p24 = 0x1p24, x1p_24 = 0x1p-24;
    double fw, z, f[20], fq[20], q[20];
    int32_t iq[20];
    int n, ih, i, j, k;
    const int jk = 4, jp = 4;
    int jx = nx - 1;
    int jv = (e0 - 3) / 24;
    if (jv < 0)
        jv = 0;
    int q0 = e0 - 24 * (jv + 1);
    j = jv - jx;
    int m = jx + jk;
    for (i = 0; i <= m; i++, j++)
        f[i] = j < 0 ? 0.0 : (double)IPIO2[j];
    for (i = 0; i <= jk; i++) {
        for (j = 0, fw = 0.0; j <= jx; j++)
            fw += x[j] * f[jx + i - j];
        q[i] = fw;
    }
    int jz = jk;
    for (;;) {
        /* distill q[] into iq[] reversingly */
        for (i = 0, j = jz, z = q[jz]; j > 0; i++, j--) {
            fw = (double)(int32_t)(x1p_24 * z);
            iq[i] = (int32_t)(z - x1p24 * fw);
            z = q[j - 1] + fw;
        }
        z = scalbn(z, q0);
        z -= 8.0 * floor(z * 0.125);
        n = (int32_t)z;
        z -= (double)n;
        ih = 0;
        if (q0 > 0) {
            i = iq[jz - 1] >> (24 - q0);
            n += i;
            iq[jz - 1] -= i << (24 - q0);
            ih = iq[jz - 1] >> (23 - q0);
        } else if (q0 == 0) {
            ih = iq[jz - 1] >> 23;
        } else if (z >= 0.5) {
            ih = 2;
        }
        if (ih > 0) {
            n += 1;
            int carry = 0;
            for (i = 0; i < jz; i++) {
                j = iq[i];
                if (carry == 0) {
                    if (j != 0) {
                        carry = 1;
                        iq[i] = 0x1000000 - j;
                    }
                } else {
                    iq[i] = 0xffffff - j;
                }
            }
            if (q0 > 0) {
                if (q0 == 1)
                    iq[jz - 1] &= 0x7fffff;
                else if (q0 == 2)
                    iq[jz - 1] &= 0x3fffff;
            }
            if (ih == 2) {
                z = 1.0 - z;
                if (carry != 0)
                    z -= scalbn(1.0, q0);
            }
        }
        if (z == 0.0) {
            j = 0;
            for (i = jz - 1; i >= jk; i--)
                j |= iq[i];
            if (j == 0) {
                for (k = 1; iq[jk - k] == 0; k++)
                    ;
                for (i = jz + 1; i <= jz + k; i++) {
                    f[jx + i] = (double)IPIO2[jv + i];
                    for (j = 0, fw = 0.0; j <= jx; j++)
                        fw += x[j] * f[jx + i - j];
                    q[i] = fw;
                }
                jz += k;
                continue;
            }
        }
        break;
    }
    /* chop off zero terms */
    if (z == 0.0) {
        jz -= 1;
        q0 -= 24;
        while (iq[jz] == 0) {
            jz--;
            q0 -= 24;
        }
    } else {
        z = scalbn(z, -q0);
        if (z >= x1p24) {
            fw = (double)(int32_t)(x1p_24 * z);
            iq[jz] = (int32_t)(z - x1p24 * fw);
            jz += 1;
            q0 += 24;
            iq[jz] = (int32_t)fw;
        } else {
            iq[jz] = (int32_t)z;
        }
    }
    fw = scalbn(1.0, q0);
    for (i = jz; i >= 0; i--) {
        q[i] = fw * (double)iq[i];
        fw *= x1p_24;
    }
    for (i = jz; i >= 0; i--) {
        for (fw = 0.0, k = 0; k <= jp && k <= jz - i; k++)
            fw += PIO2[k] * q[i + k];
        fq[jz - i] = fw;
    }
    fw = 0.0;
    for (i = jz; i >= 0; i--)
        fw += fq[i];
    y[0] = ih == 0 ? fw : -fw;
    fw = fq[0] - fw;
    for (i = 1; i <= jz; i++)
        fw += fq[i];
    y[1] = ih == 0 ? fw : -fw;
    return n & 7;
}

static const double INV_PIO2 = 6.36619772367581382433e-01, PIO2_1 = 1.57079632673412561417e+00,
                    PIO2_1T = 6.07710050650619224932e-11, PIO2_2 = 6.07710050630396597660e-11,
                    PIO2_2T = 2.02226624879595063154e-21, PIO2_3 = 2.02226624871116645580e-21,
                    PIO2_3T = 8.47842766036889956997e-32;

/* x reduced to y[0] + y[1] in [-pi/4, pi/4], with the quadrant n. */
static int rem_pio2_medium(double x, uint32_t ix, double *y) {
    const double to_int = 1.5 / 2.2204460492503131e-16;
    double f_n = x * INV_PIO2 + to_int - to_int;
    int n = (int32_t)f_n;
    double r = x - f_n * PIO2_1;
    double w = f_n * PIO2_1T;
    y[0] = r - w;
    int ex = (int)(ix >> 20);
    int ey = (int)(trig_bits(y[0]) >> 52) & 0x7ff;
    if (ex - ey > 16) {
        double t = r;
        w = f_n * PIO2_2;
        r = t - w;
        w = f_n * PIO2_2T - ((t - r) - w);
        y[0] = r - w;
        ey = (int)(trig_bits(y[0]) >> 52) & 0x7ff;
        if (ex - ey > 49) {
            t = r;
            w = f_n * PIO2_3;
            r = t - w;
            w = f_n * PIO2_3T - ((t - r) - w);
            y[0] = r - w;
        }
    }
    y[1] = (r - y[0]) - w;
    return n;
}

static int rem_pio2(double x, double *y) {
    uint64_t bits = trig_bits(x);
    int sign = (int)(bits >> 63);
    uint32_t ix = (uint32_t)(bits >> 32) & 0x7fffffff;
    if (ix <= 0x400f6a7a) {
        if ((ix & 0xfffff) == 0x921fb)
            return rem_pio2_medium(x, ix, y);
        int k = ix <= 0x4002d97c ? 1 : 2;
        double z = sign ? x + k * PIO2_1 : x - k * PIO2_1;
        y[0] = sign ? z + k * PIO2_1T : z - k * PIO2_1T;
        y[1] = sign ? (z - y[0]) + k * PIO2_1T : (z - y[0]) - k * PIO2_1T;
        return sign ? -k : k;
    }
    if (ix <= 0x401c463b) {
        if (ix == 0x4012d97c || ix == 0x401921fb)
            return rem_pio2_medium(x, ix, y);
        int k = ix <= 0x4015fdbc ? 3 : 4;
        double z = sign ? x + k * PIO2_1 : x - k * PIO2_1;
        y[0] = sign ? z + k * PIO2_1T : z - k * PIO2_1T;
        y[1] = sign ? (z - y[0]) + k * PIO2_1T : (z - y[0]) - k * PIO2_1T;
        return sign ? -k : k;
    }
    if (ix < 0x413921fb)
        return rem_pio2_medium(x, ix, y);
    if (ix >= 0x7ff00000) {
        y[0] = y[1] = x - x;
        return 0;
    }
    /* z = scalbn(|x|, -ilogb(x) + 23), split into 24-bit chunks */
    double z = trig_from_bits((bits & (~0ull >> 12)) | ((uint64_t)(0x3ff + 23) << 52));
    double tx[3], ty[2];
    for (int i = 0; i < 2; i++) {
        tx[i] = (double)(int32_t)z;
        z = (z - tx[i]) * 0x1p24;
    }
    tx[2] = z;
    int count = 3;
    while (count > 1 && tx[count - 1] == 0.0)
        count--;
    int n = rem_pio2_large(tx, ty, (int)(ix >> 20) - (0x3ff + 23), count);
    if (sign) {
        y[0] = -ty[0];
        y[1] = -ty[1];
        return -n;
    }
    y[0] = ty[0];
    y[1] = ty[1];
    return n;
}

static double kernel_sin(double x, double y, int iy) {
    const double S1 = -1.66666666666666324348e-01, S2 = 8.33333333332248946124e-03,
                 S3 = -1.98412698298579493134e-04, S4 = 2.75573137070700676789e-06,
                 S5 = -2.50507602534068634195e-08, S6 = 1.58969099521155010221e-10;
    double z = x * x, w = z * z;
    double r = S2 + z * (S3 + z * S4) + z * w * (S5 + z * S6);
    double v = z * x;
    if (iy == 0)
        return x + v * (S1 + z * r);
    return x - ((z * (0.5 * y - v * r) - y) - v * S1);
}

static double kernel_cos(double x, double y) {
    const double C1 = 4.16666666666666019037e-02, C2 = -1.38888888888741095749e-03,
                 C3 = 2.48015872894767294178e-05, C4 = -2.75573143513906633035e-07,
                 C5 = 2.08757232129817482790e-09, C6 = -1.13596475577881948265e-11;
    double z = x * x, w = z * z;
    double r = z * (C1 + z * (C2 + z * C3)) + w * w * (C4 + z * (C5 + z * C6));
    double hz = 0.5 * z;
    w = 1.0 - hz;
    return w + (((1.0 - w) - hz) + (z * r - x * y));
}

static const double TAN_T[13] = {
    3.33333333333334091986e-01, 1.33333333333201242699e-01, 5.39682539762260521377e-02, 2.18694882948595424599e-02, 8.86323982359930005737e-03, 3.59207910759131235356e-03, 1.45620945432529025516e-03, 5.88041240820264096874e-04, 2.46463134818469906812e-04, 7.81794442939557092300e-05, 7.14072491382608190305e-05, -1.85586374855275456654e-05, 2.59073051863633712884e-05,
};

static double kernel_tan(double x, double y, int odd) {
    const double pio4 = 7.85398163397448278999e-01, pio4_lo = 3.06161699786838301793e-17;
    uint32_t hx = (uint32_t)(trig_bits(x) >> 32);
    int big = (hx & 0x7fffffff) >= 0x3FE59428;
    int sign = (int)(hx >> 31);
    if (big) {
        if (sign) {
            x = -x;
            y = -y;
        }
        x = (pio4 - x) + (pio4_lo - y);
        y = 0.0;
    }
    double z = x * x, w = z * z;
    double r = TAN_T[1] + w * (TAN_T[3] + w * (TAN_T[5] + w * (TAN_T[7] + w * (TAN_T[9] + w * TAN_T[11]))));
    double v = z * (TAN_T[2] + w * (TAN_T[4] + w * (TAN_T[6] + w * (TAN_T[8] + w * (TAN_T[10] + w * TAN_T[12])))));
    double s = z * x;
    r = y + z * (s * (r + v) + y) + s * TAN_T[0];
    w = x + r;
    if (big) {
        s = 1.0 - 2.0 * odd;
        v = s - 2.0 * (x + (r - w * w / (w + s)));
        return sign ? -v : v;
    }
    if (!odd)
        return w;
    /* -1/(x + r) accurately */
    double w0 = trig_from_bits(trig_bits(w) & 0xFFFFFFFF00000000ull);
    v = r - (w0 - x);
    double a = -1.0 / w;
    double a0 = trig_from_bits(trig_bits(a) & 0xFFFFFFFF00000000ull);
    return a0 + a * (1.0 + a0 * w0 + a0 * v);
}

double sin(double x) {
    uint32_t ix = (uint32_t)(trig_bits(x) >> 32) & 0x7fffffff;
    if (ix <= 0x3fe921fb) {
        if (ix < 0x3e500000)
            return x;
        return kernel_sin(x, 0.0, 0);
    }
    if (ix >= 0x7ff00000)
        return x - x;
    double y[2];
    switch (rem_pio2(x, y) & 3) {
    case 0: return kernel_sin(y[0], y[1], 1);
    case 1: return kernel_cos(y[0], y[1]);
    case 2: return -kernel_sin(y[0], y[1], 1);
    default: return -kernel_cos(y[0], y[1]);
    }
}

double cos(double x) {
    uint32_t ix = (uint32_t)(trig_bits(x) >> 32) & 0x7fffffff;
    if (ix <= 0x3fe921fb) {
        if (ix < 0x3e46a09e)
            return 1.0;
        return kernel_cos(x, 0.0);
    }
    if (ix >= 0x7ff00000)
        return x - x;
    double y[2];
    switch (rem_pio2(x, y) & 3) {
    case 0: return kernel_cos(y[0], y[1]);
    case 1: return -kernel_sin(y[0], y[1], 1);
    case 2: return -kernel_cos(y[0], y[1]);
    default: return kernel_sin(y[0], y[1], 1);
    }
}

double tan(double x) {
    uint32_t ix = (uint32_t)(trig_bits(x) >> 32) & 0x7fffffff;
    if (ix <= 0x3fe921fb) {
        if (ix < 0x3e400000)
            return x;
        return kernel_tan(x, 0.0, 0);
    }
    if (ix >= 0x7ff00000)
        return x - x;
    double y[2];
    int n = rem_pio2(x, y);
    return kernel_tan(y[0], y[1], n & 1);
}

float sinf(float x) { return (float)sin(x); }
float cosf(float x) { return (float)cos(x); }
float tanf(float x) { return (float)tan(x); }
