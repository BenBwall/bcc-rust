/* Fixed-point particle integration with small structures passed and
 * returned by value. Every helper takes and returns `struct vec`, so the hot
 * loop is aggregate copies, member loads and stores of local structures, and
 * calls of tiny static functions. */
int printf(const char *, ...);

#ifdef BENCH_REDUCED
#define STEPS 40
#define EXPECT 2703549946u
#else
#define STEPS 300000
#define EXPECT 3613454671u
#endif

#define COUNT 16
#define LIMIT 4096

struct vec {
  int x;
  int y;
  int z;
};

struct particle {
  struct vec pos;
  struct vec vel;
};

static struct particle particles[COUNT];
static unsigned seed = 2463534242u;

static unsigned next_random(void) {
  seed ^= seed << 13;
  seed ^= seed >> 17;
  seed ^= seed << 5;
  return seed;
}

static struct vec vadd(struct vec a, struct vec b) {
  struct vec r = {a.x + b.x, a.y + b.y, a.z + b.z};
  return r;
}

static struct vec vscale(struct vec a, int s) {
  struct vec r = {(a.x * s) / 256, (a.y * s) / 256, (a.z * s) / 256};
  return r;
}

static int clamp(int v) {
  return v < -LIMIT ? -LIMIT : v > LIMIT ? LIMIT : v;
}

static struct vec vclamp(struct vec a) {
  struct vec r = {clamp(a.x), clamp(a.y), clamp(a.z)};
  return r;
}

static int vdot(struct vec a, struct vec b) {
  return a.x * b.x + a.y * b.y + a.z * b.z;
}

static struct vec kick(void) {
  unsigned r = next_random();
  struct vec k = {(int)(r & 63) - 32, (int)((r >> 6) & 63) - 32, (int)((r >> 12) & 63) - 32};
  return k;
}

int main(void) {
  for (int i = 0; i < COUNT; i++) {
    struct vec p = {i * 97 - 700, i * 31 - 250, 400 - i * 53};
    struct vec v = {0, 0, 0};
    particles[i].pos = p;
    particles[i].vel = v;
  }
  unsigned energy = 0;
  for (int step = 0; step < STEPS; step++) {
    for (int i = 0; i < COUNT; i++) {
      struct particle p = particles[i];
      struct vec spring = vscale(p.pos, -3);
      p.vel = vadd(p.vel, spring);
      p.vel = vadd(p.vel, kick());
      p.vel = vclamp(vscale(p.vel, 250));
      p.pos = vclamp(vadd(p.pos, p.vel));
      energy += (unsigned)vdot(p.vel, p.vel) >> 4;
      particles[i] = p;
    }
  }
  unsigned checksum = energy;
  for (int i = 0; i < COUNT; i++) {
    struct vec p = particles[i].pos;
    checksum = checksum * 31u + (unsigned)vdot(p, p);
  }
  printf("particles: checksum %u\n", checksum);
  return checksum == EXPECT ? 0 : 1;
}
