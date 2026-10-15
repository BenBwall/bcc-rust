/* Escape-time Mandelbrot iteration counts over a grid, with a `struct
 * complex` of two doubles passed to and returned from small helpers.
 * Floating-point multiply-add chains, a data-dependent loop exit and local
 * structure copies. */
int printf(const char *, ...);

#ifdef BENCH_REDUCED
#define WIDTH 24
#define HEIGHT 16
#define EXPECT 1501904185u
#else
#define WIDTH 600
#define HEIGHT 400
#define EXPECT 2473547029u
#endif

#define MAX_ITER 200

struct complex {
  double re;
  double im;
};

static struct complex csquare_add(struct complex z, struct complex c) {
  struct complex r = {z.re * z.re - z.im * z.im + c.re, 2.0 * z.re * z.im + c.im};
  return r;
}

static double cnorm(struct complex z) {
  return z.re * z.re + z.im * z.im;
}

static int escape(struct complex c) {
  struct complex z = {0.0, 0.0};
  int n = 0;
  while (n < MAX_ITER && cnorm(z) <= 4.0) {
    z = csquare_add(z, c);
    n++;
  }
  return n;
}

int main(void) {
  unsigned checksum = 0;
  for (int y = 0; y < HEIGHT; y++) {
    for (int x = 0; x < WIDTH; x++) {
      struct complex c = {-2.2 + 3.2 * x / WIDTH, -1.2 + 2.4 * y / HEIGHT};
      checksum = checksum * 17u + (unsigned)escape(c);
    }
  }
  printf("mandel: checksum %u\n", checksum);
  return checksum == EXPECT ? 0 : 1;
}
