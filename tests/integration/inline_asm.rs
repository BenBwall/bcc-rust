//! GNU assembly statements inside functions, compiled and run on the host
//! through LLVM and the native back end: `rdtsc`, `cpuid`, register,
//! memory, immediate, matching and flag operands, clobbers, a memory barrier
//! and `asm goto`. Direct native objects and the interpreter refuse
//! assembly.

#[cfg(test)]
#[expect(
    clippy::disallowed_types,
    clippy::disallowed_methods,
    clippy::disallowed_macros,
    reason = "Tests own process output and temporary files."
)]
mod tests {
    use std::{
        path::PathBuf,
        process::{
            Command,
            Output,
        },
        sync::atomic::{
            AtomicU32,
            Ordering,
        },
    };

    use crate::native::run_native;

    /// Exits with 0, or with the number of the first check that fails.
    const PROGRAM: &str = r#"
static unsigned long long rdtsc(void) {
    unsigned lo, hi;
    __asm__ __volatile__("rdtsc" : "=a"(lo), "=d"(hi));
    return ((unsigned long long)hi << 32) | lo;
}
static void cpuid(unsigned leaf, unsigned *a, unsigned *b, unsigned *c, unsigned *d) {
    __asm__ volatile("cpuid" : "=a"(*a), "=b"(*b), "=c"(*c), "=d"(*d) : "a"(leaf), "c"(0));
}
static int add(int x, int y) { __asm__("addl %1, %0" : "+r"(x) : "g"(y)); return x; }
static int add_immediate(int x) {
    __asm__("addl %[imm], %[x]" : [x] "+r"(x) : [imm] "i"(42) : "cc");
    return x;
}
static int in_memory(void) { int m = 5; __asm__("addl $10, %0" : "+m"(m)); return m; }
static int load(const int *p) { int r; __asm__("movl %1, %0" : "=r"(r) : "m"(*p)); return r; }
static int matching(int a) { int r; __asm__("leal 7(%1), %0" : "=r"(r) : "0"(a), "r"(a)); return r; }
static long widened(short s) { long r; __asm__("" : "=r"(r) : "0"(s)); return r; }
static int equal(int a, int b) { int eq; __asm__("cmpl %2, %1" : "=@ccz"(eq) : "r"(a), "r"(b)); return eq; }
static double fadd(double a, double b) { __asm__("addsd %1, %0" : "+x"(a) : "x"(b)); return a; }
static long swapped(long v) { __asm__("bswap %0" : "+r"(v)); return v; }
static unsigned char high(unsigned v) { unsigned char r; __asm__("movb %h1, %0" : "=q"(r) : "Q"(v)); return r; }
static int doubled(int x) {
    int r;
    __asm__("movl %1, %%ecx\n\taddl %%ecx, %%ecx\n\tmovl %%ecx, %0" : "=r"(r) : "r"(x) : "ecx");
    return r;
}
static int early(int x) {
    int r;
    __asm__("movl $1, %0\n\taddl %1, %0" : "=&r"(r) : "r"(x));
    return r;
}
static int counter;
static void increment(void) { __asm__("incl %0" : "+m"(counter)); }
static int flag;
static int barrier(void) { flag = 1; __asm__ volatile("" ::: "memory"); return flag; }
static int sign(int x) {
    __asm__ goto("testl %0, %0\n\tjz %l[zero]\n\tjs %l[negative]" :: "r"(x) : "cc" : zero, negative);
    return 1;
zero:
    return 0;
negative:
    return -1;
}
static int all_clobbered(int x) {
    if (x)
        __asm__ goto("jmp %l[taken]" ::: "rax", "rbx", "rcx", "rdx", "rsi", "rdi", "r8", "r9",
                     "r10", "r11", "r12", "r13", "r14", "r15", "cc" : taken);
    return 0;
taken:
    return 1;
}
static int internal_target;
static int dialect(void) { int r; __asm__("{movl $1, %0|mov %0, 2}" : "=r"(r)); return r; }
static int dialect_add(int x) { __asm__("{addl %1, %0|add %0, %z1%1}" : "+r"(x) : "i"(1)); return x; }
static int bare(void) { int r; __asm__("movl $%c1, %0" : "=r"(r) : "i"(42)); return r; }
static int *symbol(void) {
    int *p;
    __asm__("leaq %P1(%%rip), %0" : "=r"(p) : "i"(&internal_target));
    return p;
}
static int alternatives(void) {
    int x, y;
    __asm__("movl $1, %%eax\n\tmovl $2, %%ebx\n\tmovl %%eax, %0\n\tmovl %%ebx, %1"
            : "=a,b"(x), "=b,a"(y));
    return x * 10 + y;
}
static int wide_match(long x, long *result) {
    int out;
    __asm__("movq %q2, %1; movl $1, %0" : "=a"(out), "=&r"(*result) : "0"(x));
    return out;
}
static int wide_flag(int a) {
    __int128 eq = -1;
    __asm__("testl %1, %1" : "=@ccz"(eq) : "r"(a));
    return eq == 1;
}
static int sse_high(long x) {
    int y;
    __asm__("psrlq $32, %0" : "=x"(y) : "0"(x));
    return y;
}
static int suffix(long x) {
    int y;
    __asm__("mov%z0 $1, %0" : "=a"(y) : "0"(x));
    return y;
}
static double x87_copy(double in) {
    double out;
    __asm__("fld%z1 %1\n\tfstp%z0 %0" : "=m"(out) : "m"(in));
    return out;
}
static float x87_copy_float(float in) {
    float out;
    __asm__("fld%z1 %1\n\tfstp%z0 %0" : "=m"(out) : "m"(in));
    return out;
}
struct counter { int value; };
static int increment_struct(void) {
    struct counter c = {41};
    __asm__("inc%z0 %0" : "+m"(c));
    return c.value;
}
static int unique(void) {
    int r;
    __asm__("jmp 1f\n1:\tmovl $%=, %0" : "=r"(r));
    return r >= 0;
}
int main(void) {
    unsigned long long t0 = rdtsc(), t1 = rdtsc();
    unsigned a, b, c, d;
    int value = 99;
    if (t1 < t0) return 1;
    cpuid(0, &a, &b, &c, &d);
    if (b == 0 && c == 0 && d == 0) return 2;
    if (add(3, 4) != 7) return 3;
    if (add_immediate(1) != 43) return 4;
    if (in_memory() != 15) return 5;
    if (load(&value) != 99) return 6;
    if (matching(10) != 17) return 7;
    if (widened(-3) != -3) return 8;
    if (equal(3, 3) != 1 || equal(3, 4) != 0) return 9;
    if (fadd(1.5, 2.25) != 3.75) return 10;
    if (swapped(0x0102030405060708L) != 0x0807060504030201L) return 11;
    if (high(0x1234) != 0x12) return 12;
    if (doubled(21) != 42) return 13;
    if (early(41) != 42) return 14;
    increment(); increment();
    if (counter != 2) return 15;
    if (barrier() != 1) return 16;
    if (sign(0) != 0 || sign(5) != 1 || sign(-5) != -1) return 17;
    if (!unique()) return 18;
    if (all_clobbered(1) != 1 || all_clobbered(0) != 0) return 19;
    if (dialect() != 1 || dialect_add(41) != 42) return 20;
    if (bare() != 42) return 21;
    if (symbol() != &internal_target) return 22;
    if (alternatives() != 12) return 23;
    long whole = 0;
    if (wide_match(0x100000002L, &whole) != 1 || whole != 0x100000002L) return 24;
    if (!wide_flag(0) || wide_flag(1)) return 25;
    if (sse_high(0x500000007L) != 5) return 26;
    if (suffix(0x700000000L) != 1) return 27;
    if (x87_copy(2.5) != 2.5 || x87_copy_float(1.25f) != 1.25f) return 28;
    if (increment_struct() != 42) return 29;
    return 0;
}
"#;

    /// The host target the program runs on, if any.
    fn host() -> Option<&'static str> {
        if !cfg!(target_arch = "x86_64") {
            return None;
        }
        if cfg!(all(windows, target_env = "gnu")) {
            Some("--target=x86_64-w64-windows-gnu")
        } else if cfg!(all(windows, target_env = "msvc")) {
            Some("--target=x86_64-pc-windows-msvc")
        } else if cfg!(all(target_os = "linux", target_env = "musl")) {
            Some("--target=x86_64-unknown-linux-musl")
        } else if cfg!(target_os = "linux") {
            Some("--target=x86_64-unknown-linux-gnu")
        } else {
            None
        }
    }

    #[test]
    fn programs_run_every_operand_kind_on_both_back_ends() {
        let Some(target) = host() else {
            eprintln!("skipping: the host is not an x86-64 target bcc supports");
            return;
        };
        for flags in [
            &["--backend=llvm"][..],
            &["--backend=llvm", "-O2"][..],
            &["--backend=llvm", "--opt=bcc"][..],
            &["--backend=native"][..],
            &["--backend=native", "--opt=bcc"][..],
        ] {
            let executable = temporary("inline-asm.exe");
            let built = bcc(
                PROGRAM,
                &[&[target, "-o", executable.to_str().unwrap()][..], flags].concat(),
            );
            assert!(built.status.success(), "{flags:?}: {}", text(&built.stderr));
            let run = run_native(&mut Command::new(&executable)).unwrap();
            assert_eq!(run.status.code(), Some(0), "{flags:?}");
            std::fs::remove_file(executable).unwrap();
        }
    }

    /// The template reaches LLVM with Clang's constraints: results for
    /// register outputs, `elementtype` memory operands with a `+m` one also
    /// read, and the flags every x86 statement clobbers.
    #[test]
    fn llvm_inline_asm_uses_clangs_constraints() {
        let source = "int f(int x, int *p) { int r; __asm__(\"op %0, %1, %2\" : \"=&r\"(r), \
                      \"+m\"(*p) : \"a\"(x) : \"memory\", \"rcx\"); return r; }";
        let output = bcc(
            source,
            &["--target=x86_64-unknown-linux-gnu", "--emit=llvm"],
        );
        assert!(output.status.success(), "{}", text(&output.stderr));
        let llvm = text(&output.stdout);
        assert!(
            llvm.contains(
                "call i32 asm \"op $0, $1, $2\", \
                 \"=&r,=*m,{ax},*m,~{dirflag},~{fpsr},~{flags},~{memory},~{cx}\"(ptr \
                 elementtype(i32)"
            ),
            "{llvm}"
        );
    }

    /// LLVM gets exactly the registers a constraint allows: `Q` and one
    /// register have LLVM constraints, a set like `ab` has none and is
    /// refused rather than widened.
    #[test]
    fn llvm_register_sets_are_exact_or_refused() {
        let output = bcc(
            "int f(void) { int x, y; __asm__(\"\" : \"=Q\"(x), \"=D\"(y)); return x + y; }",
            &["--target=x86_64-unknown-linux-gnu", "--emit=llvm"],
        );
        assert!(output.status.success(), "{}", text(&output.stderr));
        assert!(
            text(&output.stdout).contains("\"=Q,={di},"),
            "{}",
            text(&output.stdout)
        );
        let output = bcc(
            "int f(void) { int x; __asm__(\"\" : \"=ab\"(x)); return x; }",
            &["--target=x86_64-unknown-linux-gnu", "--emit=llvm"],
        );
        assert_eq!(output.status.code(), Some(1));
        assert!(
            text(&output.stderr)
                .contains("inline assembly operand restricted to the general registers {rax,rbx}"),
            "{}",
            text(&output.stderr)
        );
    }

    /// An initialized local incremented through `+m` is read by the
    /// statement (Clang's `=*m,*m`), so optimization keeps its initial
    /// value: the program returns 15 at O2, through LLVM's and bcc's
    /// optimizers.
    #[test]
    fn optimized_update_memory_operands_are_read() {
        let source = "__attribute__((noinline)) int f(void) { int m = 5; __asm__(\"addl $10, %0\" \
                      : \"+m\"(m)); return m; }\nint main(void) { return f() != 15; }\n";
        let output = bcc(
            source,
            &["--target=x86_64-unknown-linux-gnu", "--emit=llvm"],
        );
        assert!(output.status.success(), "{}", text(&output.stderr));
        let llvm = text(&output.stdout);
        assert!(
            llvm.contains(
                "\"=*m,*m,~{dirflag},~{fpsr},~{flags}\"(ptr elementtype(i32) %slot0, ptr \
                 elementtype(i32) %slot0)"
            ),
            "{llvm}"
        );
        let Some(target) = host() else {
            return;
        };
        for flags in [&["-O2"][..], &["-O2", "--opt=bcc"][..]] {
            let executable = temporary("inline-asm-update.exe");
            let built = bcc(
                source,
                &[&[target, "-o", executable.to_str().unwrap()][..], flags].concat(),
            );
            assert!(built.status.success(), "{flags:?}: {}", text(&built.stderr));
            let run = run_native(&mut Command::new(&executable)).unwrap();
            assert_eq!(run.status.code(), Some(0), "{flags:?}");
            std::fs::remove_file(executable).unwrap();
        }
    }

    /// The native listing writes the substituted template between `#APP`
    /// and `#NO_APP`; direct objects cannot encode it, so they are refused
    /// on request and the listing is assembled otherwise.
    #[test]
    fn native_listings_carry_the_template_and_direct_objects_refuse_it() {
        let source = "int f(int x) { __asm__(\"addl $1, %0\" : \"+r\"(x)); return x; }";
        let output = bcc(
            source,
            &[
                "--target=x86_64-unknown-linux-gnu",
                "--backend=native",
                "--emit=asm",
            ],
        );
        assert!(output.status.success(), "{}", text(&output.stderr));
        let listing = text(&output.stdout);
        assert!(listing.contains("#APP\n\taddl $1, %"), "{listing}");
        for target in [
            "--target=x86_64-unknown-linux-gnu",
            "--target=x86_64-w64-windows-gnu",
        ] {
            let object = temporary("inline-asm-direct.o");
            let refused = bcc(
                source,
                &[
                    target,
                    "--backend=native",
                    "--native-object",
                    "-c",
                    "-o",
                    object.to_str().unwrap(),
                ],
            );
            assert_eq!(refused.status.code(), Some(1), "{target}");
            assert!(
                text(&refused.stderr).contains("--native-object cannot encode inline assembly"),
                "{target}: {}",
                text(&refused.stderr)
            );
            assert!(!object.exists(), "{target}");
        }
    }

    #[test]
    fn the_interpreter_refuses_inline_assembly() {
        let output = bcc(
            "int main(void) { __asm__(\"nop\"); return 0; }",
            &["--target=x86_64-unknown-linux-gnu", "--interpret"],
        );
        assert_eq!(output.status.code(), Some(70));
        assert!(
            text(&output.stderr).contains(
                "unsupported by the interpreter: inline assembly, which only compiled code can \
                 run; compile it with -o instead"
            ),
            "{}",
            text(&output.stderr)
        );
    }

    fn bcc(source: &str, flags: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_bcc-rust"))
            .args(flags)
            .args(["--input", source])
            .env("NO_COLOR", "1")
            .env_remove("CLICOLOR_FORCE")
            .output()
            .unwrap()
    }

    /// Output bytes as text, with Windows line ends made plain.
    fn text(bytes: &[u8]) -> String {
        String::from_utf8_lossy(bytes).replace("\r\n", "\n")
    }

    /// A fresh path in the system temporary directory.
    fn temporary(name: &str) -> PathBuf {
        static NEXT: AtomicU32 = AtomicU32::new(0);
        std::env::temp_dir().join(format!(
            "bcc-inline-asm-{}-{}-{name}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ))
    }
}
