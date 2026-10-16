//! GNU labels as values and computed goto, end to end: label tables in
//! static data as in Lua's `lvm.c`, label differences added back to a base
//! label, threaded dispatch with SSA values live across every indirect edge,
//! local labels, and jumps out of statement expressions and VLA scopes. Each
//! program interprets to status 0 in every stage, and every native arm
//! prints and returns what Clang's build of the same source does.
//! GNU extension: GCC "Labels as Values",
//! <https://gcc.gnu.org/onlinedocs/gcc/Labels-as-Values.html>.

use super::*;

/// The conformance reproducer: a static table of label addresses.
const TABLE: &str = "int main(void) {
    static const void *const table[] = { &&a, &&b };
    int i = 0;
    goto *table[1];
a:  i = 1;
b:  return i - 0;
}
";

/// A register machine dispatched as Lua's `luaV_execute` does with
/// `LUA_USE_JUMPTABLE`: `static const void *const disptab[]` and a
/// `goto *disptab[op]` after every instruction.
const LUA_DISPATCH: &str = "int printf(const char *, ...);
enum { OP_LOADK, OP_ADD, OP_MUL, OP_DEC, OP_JNZ, OP_PRINT, OP_HALT };
typedef struct { unsigned char op, a, b; } Insn;
static int execute(const Insn *code, int *regs) {
    static const void *const disptab[] = {
        &&L_OP_LOADK, &&L_OP_ADD, &&L_OP_MUL, &&L_OP_DEC, &&L_OP_JNZ, &&L_OP_PRINT, &&L_OP_HALT
    };
    const Insn *pc = code;
    Insn i;
    int steps = 0;
#define vmfetch() (i = *pc++, steps++)
#define vmdispatch(o) goto *disptab[o]
#define vmbreak vmfetch(); vmdispatch(i.op)
    vmfetch();
    vmdispatch(i.op);
L_OP_LOADK: regs[i.a] = i.b; vmbreak;
L_OP_ADD: regs[i.a] += regs[i.b]; vmbreak;
L_OP_MUL: regs[i.a] *= regs[i.b]; vmbreak;
L_OP_DEC: regs[i.a]--; vmbreak;
L_OP_JNZ: if (regs[i.a]) pc -= i.b; vmbreak;
L_OP_PRINT: printf(\"r%d=%d\\n\", i.a, regs[i.a]); vmbreak;
L_OP_HALT: return steps;
}
int main(void) {
    static const Insn program[] = {
        {OP_LOADK, 0, 1}, {OP_LOADK, 1, 5}, {OP_LOADK, 2, 2},
        {OP_MUL, 0, 2}, {OP_PRINT, 0, 0}, {OP_DEC, 1, 0}, {OP_JNZ, 1, 4},
        {OP_ADD, 0, 2}, {OP_PRINT, 0, 0}, {OP_HALT, 0, 0},
    };
    int regs[3] = {0};
    int steps = execute(program, regs);
    printf(\"steps=%d\\n\", steps);
    return regs[0] != 34 || steps != 26;
}
";

/// Position-independent dispatch: label differences in `int` and `long`
/// static tables added back to a base label, and differences computed at
/// run time, narrowed to `short` and added back, as in GCC's 920302-1.
const LABEL_DIFFERENCES: &str = "int printf(const char *, ...);
static int classify(int k) {
    static const int offsets[] = { &&zero - &&zero, &&one - &&zero, &&many - &&zero };
    static const long wide[] = { &&zero - &&many, &&one - &&many, &&many - &&many };
    int which = k < 2 ? k : 2;
    void *target = (k & 1) ? &&zero + offsets[which] : &&many + wide[which];
    goto *target;
zero: return 100;
one: return 200;
many: return 300 + k;
}
static int runtime(int k) {
    void *base = &&a;
    short delta[3];
    void *labels[3] = { &&a, &&b, &&c };
    for (int i = 0; i < 3; i++)
        delta[i] = (short)((char *)labels[i] - (char *)base);
    goto *(base + delta[k]);
a: return 1;
b: return 2;
c: return 3;
}
int main(void) {
    int sum = 0;
    for (int k = 0; k < 5; k++) sum += classify(k);
    for (int k = 0; k < 3; k++) sum += runtime(k) * 1000;
    printf(\"%d\\n\", sum);
    return sum != 7209;
}
";

/// A threaded loop: SSA locals live across every indirect edge, a switch in
/// a handler, targets chosen by conditionals, and a jump out of a VLA scope,
/// whose storage, as in Clang, is released when the function returns.
const THREADED: &str = "int printf(const char *, ...);
int run(int n) {
    void *next = &&start;
    long acc = 0;
    int i = 0, j = 7;
    goto *next;
start:
    if (i >= n) goto done;
    {
        int vla[(i % 4) + 1];
        vla[0] = i;
        acc += vla[0];
        next = (i & 1) ? &&odd : &&even;
        goto *next;
    }
odd:
    switch (i % 3) {
    case 0: acc += 3; break;
    case 1: acc -= 1; break;
    default: j++;
    }
    i++;
    goto *(&&start);
even:
    acc *= 2;
    i++;
    goto *(i < n ? &&start : &&done);
done:
    return (int)(acc % 100000) + j;
}
int main(void) {
    int r = run(40);
    printf(\"%d\\n\", r);
    return r != run(40);
}
";

/// Label addresses in a local table, a computed goto that never runs, a
/// label whose address is taken but that nothing reaches, a computed goto
/// out of a statement expression and GNU local labels. LLVM leaves equality
/// between two label addresses undefined, so only null comparisons appear.
const MISCELLANEOUS: &str = "int printf(const char *, ...);
struct entry { const char *name; void *label; };
int f(int c) {
    struct entry table[2] = { { \"first\", &&first }, { \"second\", &&second } };
    void *p = c ? table[0].label : table[1].label;
    printf(\"%s\\n\", table[!c].name);
    if (p != 0) goto *p;
    return -1;
first: return 1;
second: return 2;
}
int g(int x) {
    void *p = &&skip;
    if (x > 100) goto *p;
    return x + 1;
skip:
    return 0;
}
int h(int x) {
    void *unused = &&never;
    (void)unused;
    return x * 2;
never:
    return -1;
}
int k(int x) {
    void *out = &&out;
    int r = ({ int y = x * 3; if (y > 10) goto *out; y; });
    return r;
out:
    return 99;
}
int local(int x) {
    __label__ one, two;
    static void *jtab[2];
    jtab[0] = &&one;
    jtab[1] = &&two;
    goto *jtab[x];
one: return 10;
two: return 20;
}
int main(void) {
    int r = f(1) + 10 * f(0) + g(4) + h(3) + k(2) + k(5) + local(1);
    printf(\"%d\\n\", r);
    return r != 1 + 20 + 5 + 6 + 6 + 99 + 20;
}
";

/// GNU `void *` arithmetic, which position-independent dispatch relies on:
/// subscripts, address constants and constant differences step by bytes.
/// GNU extension: GCC "Pointer Arith".
const VOID_POINTERS: &str = "int arr[8];
static long d = (void *)&arr[3] - (void *)&arr[0];
static void *e = (void *)arr + 4;
static void *g = &((void *)arr)[2];
int f(void *p) {
    void *q = &p[2];
    (void)p[1];
    unsigned long s = sizeof(p[1]);
    q += 1;
    q -= 1;
    return (int)s + (int)((char *)q - (char *)p) + (int)d + (int)((char *)e - (char *)g);
}
int main(void) { return f(arr) != 1 + 2 + 12 + 2; }
";

/// The optimizer around labels: a labeled arm of `&&` whose address is only
/// stored (condition merging must keep it), a dispatch loop with a structure
/// copied and updated across indirect edges (scalar replacement), and a
/// labeled test block reached by direct jumps with known flags and by the
/// indirect branch (constant threading on the direct edges only).
const OPTIMIZED_DISPATCH: &str = "int printf(const char *, ...);
static void *saved;
static int tested(int x, int y) {
    saved = &&inner;
    if (x) {
    inner:
        if (y > 3) return 1;
    }
    return 0;
}
struct pair { int x, y; long z; };
static int dispatch(const unsigned char *code, int *mem) {
    static void *const ops[] = { &&op_add, &&op_swap, &&op_test, &&op_halt, &&check };
    struct pair p = { 1, 2, 3 }, q;
    int flag = 0, pc = 0, acc = 0;
    goto *ops[code[pc++]];
op_add:
    flag = 1;
    p.x += mem[0];
    p.z += p.y;
    if (p.x & 4) goto op_test;
    goto check;
op_swap:
    flag = (p.x & 1) && (p.y > 3);
    q = p;
    p.x = q.y;
    p.y = q.x + (mem[1] & 3);
    goto *ops[code[pc++]];
op_test:
    flag = 0;
    mem[0] += p.y;
    acc += mem[0];
    if (acc & 8) goto op_add;
    goto check;
check:
    if (flag && p.y) {
        acc += p.x;
        mem[1] ^= acc;
    } else {
        acc -= p.y;
    }
    if (acc > 100000 || acc < -100000) acc = 0;
    goto *ops[code[pc++]];
op_halt:
    return acc * 7 + p.x + p.y + (int)p.z + mem[0] + mem[1] + flag;
}
int main(void) {
    int mem[2] = { 3, 9 };
    static const unsigned char code[] = { 0, 1, 4, 2, 0, 4, 2, 1, 1, 0, 0, 2, 1, 4, 2, 0, 1, 3 };
    int r = dispatch(code, mem);
    int t = tested(1, 5) * 100 + tested(1, 2) * 10 + tested(0, 9);
    printf(\"%d %d %d %d\\n\", r, mem[0], mem[1], t);
    return r != 4121 || t != 100 || saved == 0;
}
";

const PROGRAMS: [(&str, &str); 7] = [
    ("table", TABLE),
    ("lua", LUA_DISPATCH),
    ("differences", LABEL_DIFFERENCES),
    ("threaded", THREADED),
    ("miscellaneous", MISCELLANEOUS),
    ("void_pointers", VOID_POINTERS),
    ("optimized_dispatch", OPTIMIZED_DISPATCH),
];

#[test]
fn computed_goto_programs_interpret_in_every_stage() {
    for (name, source) in PROGRAMS {
        for target in [Target::LinuxGnu, Target::WindowsGnu] {
            let mut outputs = Vec::new();
            for (optimize, profile) in STAGES {
                let (status, output) = with_module(source, target, optimize, profile, interpret);
                assert_eq!(
                    status, 0,
                    "{name} {target:?} {optimize} {profile:?}\n{output}"
                );
                outputs.push(output);
            }
            assert!(
                outputs.windows(2).all(|pair| pair[0] == pair[1]),
                "{name} {target:?}: {outputs:?}"
            );
        }
    }
}

#[test]
fn computed_goto_native_arms_match_clang() {
    let Some(target) = host() else {
        eprintln!("skipping: the host is not a supported target");
        return;
    };
    let toolchain = match Toolchain::bundled(target) {
        | Ok(toolchain) => toolchain,
        | Err(error) => {
            eprintln!("skipping: {error}");
            return;
        },
    };
    let directory = TempDir::new("computed-goto");
    thread::scope(|scope| {
        let handles: Vec<_> = PROGRAMS
            .iter()
            .map(|&(name, source)| {
                let directory = &directory;
                let toolchain = &toolchain;
                scope.spawn(move || {
                    let input = directory.join(format!("{name}.c"));
                    std::fs::write(&input, source).unwrap();
                    let mut results = Vec::new();
                    for opt in [OptLevel::O0, OptLevel::O2] {
                        let clang = directory.join(format!("{name}-clang-{opt:?}.exe"));
                        let options = LlvmOptions {
                            target,
                            opt: Some(opt),
                            disable_llvm_passes: false,
                            kind: OutputKind::Executable,
                        };
                        let output = toolchain.command(&input, &clang, options).output().unwrap();
                        assert!(
                            output.status.success(),
                            "{name}: {}",
                            String::from_utf8_lossy(&output.stderr)
                        );
                        results.push((format!("clang {opt:?}"), run_native(&clang)));
                    }
                    for arm in ARMS {
                        let executable = directory.join(format!("{name}-{}.exe", arm.name));
                        let options = LlvmOptions {
                            opt: Some(arm.opt),
                            disable_llvm_passes: arm.disable_llvm_passes,
                            target,
                            kind: OutputKind::Executable,
                        };
                        let interpreted =
                            with_module(source, target, arm.bcc, Profile::PostAbi, |module, _| {
                                compile_module(module, &executable, options)
                                    .unwrap_or_else(|error| panic!("{name} {}: {error}", arm.name));
                                interpret(module, target)
                            });
                        results.push((format!("{} interpreted", arm.name), interpreted));
                        results.push((arm.name.to_owned(), run_native(&executable)));
                    }
                    (name, results)
                })
            })
            .collect();
        for handle in handles {
            let (name, results) = handle.join().unwrap();
            let (_, expected) = &results[0];
            assert_eq!(expected.0, 0, "{name} clang");
            for (arm, actual) in &results {
                assert_eq!(actual, expected, "{name} {arm}");
            }
        }
    });
}
