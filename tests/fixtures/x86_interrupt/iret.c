/* An interrupt handler entered as the processor enters one, on a frame that
   leads back here through iretq: control, the stack pointer, the flags and
   every register come back, as the handler found them. */
typedef unsigned long uword_t;

struct frame {
  uword_t ip, cs, flags, sp, ss;
};

/* enter(handler, error, has_error, in, out): loads the 15 general registers
   other than rsp from `in`, and with SSE their 16 XMM neighbours, sets the
   direction flag, pushes what the processor pushes, and jumps to `handler`.
   Where iretq lands it writes them to `out`, then the flags. */
void enter(void (*handler)(), uword_t error, uword_t has_error, const uword_t *in, uword_t *out);
__asm__(
    ".text\n"
    ".globl enter\n"
    ".type enter,@function\n"
    "enter:\n"
    "  pushq %rbx\n  pushq %rbp\n  pushq %r12\n  pushq %r13\n  pushq %r14\n  pushq %r15\n"
    "  pushq %r8\n"
    "  movq %rsp, %rax\n"
    "  subq $128, %rsp\n"
    "  andq $-16, %rsp\n"
    "  movq %ss, %r10\n"
    "  pushq %r10\n"
    "  pushq %rax\n"
    "  pushfq\n"
    "  orq $0x400, (%rsp)\n"
    "  movq %cs, %r10\n"
    "  pushq %r10\n"
    "  leaq 1f(%rip), %r10\n"
    "  pushq %r10\n"
    "  testq %rdx, %rdx\n"
    "  jz 2f\n"
    "  pushq %rsi\n"
    "2:\n"
    "  movq %rdi, %r13\n"
    "  movq %rcx, %r15\n"
    "  movdqu 120(%r15), %xmm0\n  movdqu 136(%r15), %xmm1\n"
    "  movdqu 120(%r15), %xmm7\n  movdqu 136(%r15), %xmm15\n"
    "  pushq %r13\n"
    "  movq 0(%r15), %rax\n  movq 8(%r15), %rcx\n  movq 16(%r15), %rdx\n"
    "  movq 24(%r15), %rbx\n  movq 32(%r15), %rbp\n  movq 40(%r15), %rsi\n"
    "  movq 48(%r15), %rdi\n  movq 56(%r15), %r8\n  movq 64(%r15), %r9\n"
    "  movq 72(%r15), %r10\n  movq 80(%r15), %r11\n  movq 88(%r15), %r12\n"
    "  movq 96(%r15), %r13\n  movq 104(%r15), %r14\n  movq 112(%r15), %r15\n"
    "  std\n"
    "  retq\n"
    "1:\n"
    "  pushfq\n"
    "  cld\n"
    "  pushq %rax\n"
    "  movq 16(%rsp), %rax\n"
    "  movq %rcx, 8(%rax)\n  movq %rdx, 16(%rax)\n  movq %rbx, 24(%rax)\n"
    "  movq %rbp, 32(%rax)\n  movq %rsi, 40(%rax)\n  movq %rdi, 48(%rax)\n"
    "  movq %r8, 56(%rax)\n  movq %r9, 64(%rax)\n  movq %r10, 72(%rax)\n"
    "  movq %r11, 80(%rax)\n  movq %r12, 88(%rax)\n  movq %r13, 96(%rax)\n"
    "  movq %r14, 104(%rax)\n  movq %r15, 112(%rax)\n"
    "  popq %rcx\n"
    "  movq %rcx, 0(%rax)\n"
    "  popq %rcx\n"
    "  movq %rcx, 120(%rax)\n"
    "  movdqu %xmm0, 128(%rax)\n  movdqu %xmm1, 144(%rax)\n"
    "  movdqu %xmm7, 160(%rax)\n  movdqu %xmm15, 176(%rax)\n"
    "  popq %r8\n"
    "  popq %r15\n  popq %r14\n  popq %r13\n  popq %r12\n  popq %rbp\n  popq %rbx\n"
    "  retq\n"
    ".size enter, .-enter\n");

volatile uword_t seen_error, seen_ip, seen_cs, seen_sp, seen_flags, seen_df;
volatile int entered, aligned_ok;

__attribute__((noinline)) int helper(volatile int *a, int n) {
  int sum = 0;
  for (int i = 0; i < n; i++)
    sum += a[i] * (i + 3);
  return sum;
}

static void note(struct frame *f) {
  /* Past the red zone, where a leaf keeps its locals below %rsp. */
  __asm__ volatile("leaq -128(%%rsp), %%rsp\n\tpushfq\n\tpopq %0\n\tleaq 128(%%rsp), %%rsp"
                   : "=r"(seen_df));
  seen_ip = f->ip;
  seen_cs = f->cs;
  seen_sp = f->sp;
  seen_flags = f->flags;
  entered++;
}

__attribute__((interrupt)) void plain(struct frame *f) { note(f); }

__attribute__((interrupt)) void exception(struct frame *f, uword_t error) {
  note(f);
  seen_error = error;
}

__attribute__((interrupt)) void calling(struct frame *f, uword_t error) {
  volatile int a[20];
  for (int i = 0; i < 20; i++)
    a[i] = i * (int) error;
  aligned_ok = helper(a, 20) == 0 ? 0 : 1;
  note(f);
  seen_error = error;
}

/* Clobbers every register a call would not. */
__attribute__((interrupt)) void clobbering(struct frame *f) {
  __asm__ volatile("xorl %%ebx, %%ebx\n\txorl %%r12d, %%r12d\n\txorl %%r13d, %%r13d\n\t"
                   "xorl %%r14d, %%r14d\n\txorl %%r15d, %%r15d"
                   : : : "rbx", "r12", "r13", "r14", "r15", "cc");
  note(f);
}

typedef int aligned64 __attribute__((aligned(64)));

__attribute__((interrupt)) void over_aligned(struct frame *f, uword_t error) {
  aligned64 v = 5;
  volatile int *p = &v;
  aligned_ok = ((uword_t) p & 63) == 0 && *p == 5;
  note(f);
  seen_error = error;
}

__attribute__((interrupt)) void over_aligned_plain(struct frame *f) {
  aligned64 v = 7;
  volatile int *p = &v;
  aligned_ok = ((uword_t) p & 63) == 0 && *p == 7;
  note(f);
}

__attribute__((no_caller_saved_registers)) void saving(void) {
  volatile int a[4] = {1, 2, 3, 4};
  entered += helper(a, 4) > 0;
}

__attribute__((no_caller_saved_registers)) long saving_value(long x, long y) { return x * 3 + y; }

/* The caller keeps a value in a register a C call would change. */
__asm__(
    ".text\n"
    ".globl check_saving\n"
    ".type check_saving,@function\n"
    "check_saving:\n"
    "  pushq %rbx\n"
    "  movabsq $0x0123456789abcdef, %rcx\n  movq %rcx, %rdx\n  movq %rcx, %rsi\n"
    "  movq %rcx, %rdi\n  movq %rcx, %r8\n  movq %rcx, %r9\n  movq %rcx, %r10\n  movq %rcx, %r11\n"
    "  movq %rcx, %rax\n"
    "  callq saving\n"
    "  xorl %eax, %eax\n"
    "  movabsq $0x0123456789abcdef, %rbx\n"
    "  cmpq %rbx, %rcx\n  jne 3f\n  cmpq %rbx, %rdx\n  jne 3f\n  cmpq %rbx, %rsi\n  jne 3f\n"
    "  cmpq %rbx, %rdi\n  jne 3f\n  cmpq %rbx, %r8\n  jne 3f\n  cmpq %rbx, %r9\n  jne 3f\n"
    "  cmpq %rbx, %r10\n  jne 3f\n  cmpq %rbx, %r11\n  jne 3f\n"
    "  movl $1, %eax\n"
    "3:\n"
    "  popq %rbx\n"
    "  retq\n"
    ".size check_saving, .-check_saving\n");
int check_saving(void);

extern int printf(const char *, ...);

static int run(const char *name, void (*handler)(), uword_t error, uword_t has_error) {
  uword_t in[19], out[24];
  int bad = 0;
  for (int i = 0; i < 19; i++)
    in[i] = 0x1111111111111111UL * (i + 1);
  entered = 0;
  seen_error = 0;
  aligned_ok = -1;
  enter(handler, error, has_error, in, out);
  for (int i = 0; i < 15; i++)
    if (out[i] != in[i]) {
      printf("%s: register %d is %lx, not %lx\n", name, i, out[i], in[i]);
      bad = 1;
    }
#ifdef XMM_PRESERVED
  if (out[16] != in[15] || out[17] != in[16] || out[18] != in[17] || out[19] != in[18]
      || out[20] != in[15] || out[21] != in[16] || out[22] != in[17] || out[23] != in[18]) {
    printf("%s: XMM registers changed\n", name);
    bad = 1;
  }
#endif
  if (entered != 1) {
    printf("%s: entered %d times\n", name, entered);
    bad = 1;
  }
  if (has_error && seen_error != error) {
    printf("%s: error code %lx\n", name, seen_error);
    bad = 1;
  }
  if (!(out[15] & 0x400)) {
    printf("%s: iret did not restore the direction flag\n", name);
    bad = 1;
  }
  if (seen_df & 0x400) {
    printf("%s: the handler ran with the direction flag set\n", name);
    bad = 1;
  }
  if (aligned_ok == 0) {
    printf("%s: a local lost its alignment\n", name);
    bad = 1;
  }
  return bad;
}

int main(void) {
  int bad = 0;
  bad |= run("plain", plain, 0, 0);
  bad |= run("exception", exception, 0x1234, 1);
  bad |= run("calling", calling, 3, 1);
  bad |= run("clobbering", clobbering, 0, 0);
  bad |= run("over_aligned", over_aligned, 9, 1);
  bad |= run("over_aligned_plain", over_aligned_plain, 0, 0);
  if (!check_saving()) {
    printf("no_caller_saved_registers: a register changed\n");
    bad = 1;
  }
  if (saving_value(4, 5) != 17) {
    printf("no_caller_saved_registers: wrong value\n");
    bad = 1;
  }
  printf(bad ? "FAILED\n" : "ok\n");
  return bad;
}
