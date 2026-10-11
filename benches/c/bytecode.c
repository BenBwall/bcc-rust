/* A register bytecode interpreter running a nested-loop program. The
 * dispatch loop is an if-chain over the opcode, so the hot path is loads of
 * struct fields, compare-and-branch and indexed register access. */
int printf(const char *, ...);

#ifdef BENCH_REDUCED
#define OUTER 40
#define INNER 60
#define EXPECT 426517009u
#else
#define OUTER 14000
#define INNER 3000
#define EXPECT 873684033u
#endif

enum { LOADI, ADD, MUL, XOR, ADDI, JNZ, HALT };

struct instruction {
  int op;
  int a;
  int b;
  int c;
};

static struct instruction code[16];
static unsigned regs[8];

static void run(void) {
  int pc = 0;
  for (;;) {
    struct instruction *i = &code[pc++];
    if (i->op == LOADI) {
      regs[i->a] = (unsigned)i->b;
    } else if (i->op == ADD) {
      regs[i->a] = regs[i->b] + regs[i->c];
    } else if (i->op == MUL) {
      regs[i->a] = regs[i->b] * regs[i->c];
    } else if (i->op == XOR) {
      regs[i->a] = regs[i->b] ^ regs[i->c];
    } else if (i->op == ADDI) {
      regs[i->a] = regs[i->b] + (unsigned)i->c;
    } else if (i->op == JNZ) {
      if (regs[i->a] != 0) pc = i->b;
    } else {
      return;
    }
  }
}

static void emit(int at, int op, int a, int b, int c) {
  code[at].op = op;
  code[at].a = a;
  code[at].b = b;
  code[at].c = c;
}

int main(void) {
  /* r0 accumulator, r1 outer counter, r2 inner counter, r3 multiplier,
   * r4 scratch. */
  emit(0, LOADI, 0, 1, 0);
  emit(1, LOADI, 1, OUTER, 0);
  emit(2, LOADI, 3, 40503, 0);
  /* outer: */
  emit(3, LOADI, 2, INNER, 0);
  /* inner: */
  emit(4, MUL, 4, 0, 3);
  emit(5, ADD, 0, 4, 1);
  emit(6, XOR, 0, 0, 2);
  emit(7, ADDI, 2, 2, -1);
  emit(8, JNZ, 2, 4, 0);
  emit(9, ADDI, 1, 1, -1);
  emit(10, JNZ, 1, 3, 0);
  emit(11, HALT, 0, 0, 0);
  run();
  unsigned checksum = regs[0];
  printf("bytecode: checksum %u\n", checksum);
  return checksum == EXPECT ? 0 : 1;
}
