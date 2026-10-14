/* A stack-machine interpreter with a `switch` dispatch over a byte-coded
 * program, a local operand stack and local variables. A jump table or a
 * compare tree, and loads and stores of local arrays at a variable top of
 * stack. */
int printf(const char *, ...);

#ifdef BENCH_REDUCED
#define ITERATIONS 300
#define EXPECT 388058584u
#else
#define ITERATIONS 1000000
#define EXPECT 166991880u
#endif

enum { PUSH, LOAD, STORE, ADD, SUB, MUL, AND, SHR, DUP, JNZ, JMP, HALT };

static unsigned char program[64];
static int length;

static void op(int code) {
  program[length++] = (unsigned char)code;
}

static void op1(int code, int arg) {
  op(code);
  program[length++] = (unsigned char)arg;
}

static unsigned execute(unsigned start) {
  unsigned stack[16];
  unsigned vars[4] = {start, 0, 0, 0};
  int sp = 0;
  int pc = 0;
  for (;;) {
    switch (program[pc++]) {
    case PUSH: stack[sp++] = program[pc++]; break;
    case LOAD: stack[sp++] = vars[program[pc++]]; break;
    case STORE: vars[program[pc++]] = stack[--sp]; break;
    case ADD: sp--; stack[sp - 1] += stack[sp]; break;
    case SUB: sp--; stack[sp - 1] -= stack[sp]; break;
    case MUL: sp--; stack[sp - 1] *= stack[sp]; break;
    case AND: sp--; stack[sp - 1] &= stack[sp]; break;
    case SHR: sp--; stack[sp - 1] >>= stack[sp]; break;
    case DUP: stack[sp] = stack[sp - 1]; sp++; break;
    case JNZ: {
      int target = program[pc++];
      if (stack[--sp] != 0) pc = target;
      break;
    }
    case JMP: pc = program[pc]; break;
    default: return vars[1] ^ vars[2];
    }
  }
}

int main(void) {
  /* vars: 0 = x, 1 = acc, 2 = mix, 3 = counter */
  op1(PUSH, 7);
  op1(STORE, 3);
  int loop = length;
  op1(LOAD, 0);
  op1(PUSH, 3);
  op(MUL);
  op1(PUSH, 1);
  op(ADD);
  op(DUP);
  op1(STORE, 0);
  op1(LOAD, 1);
  op(ADD);
  op1(STORE, 1);
  op1(LOAD, 0);
  op1(PUSH, 5);
  op(SHR);
  op1(LOAD, 2);
  op(SUB);
  op1(STORE, 2);
  op1(LOAD, 3);
  op1(PUSH, 1);
  op(SUB);
  op(DUP);
  op1(STORE, 3);
  op1(JNZ, loop);
  op(HALT);
  unsigned checksum = 0;
  for (unsigned i = 0; i < ITERATIONS; i++) checksum = checksum * 3u + execute(i);
  printf("switchvm: checksum %u\n", checksum);
  return checksum == EXPECT ? 0 : 1;
}
