//! Conformance with Clang: a bcc-compiled C file and a Clang-compiled C file
//! exchange structures and unions by value in both directions, linked into
//! one executable, which exits with 0 when every value arrived intact or
//! with the number of the first check that failed.
//!
//! The bcc side goes through lowering, the optimizer, ABI lowering and the
//! LLVM back end; the Clang side is compiled by the bundled clang from C.
//! Both are built at `-O0` and `-O2`. The Windows GNU host runs its
//! executables directly; System V executables are linked freestanding and
//! run under WSL when it is available. A missing toolchain or WSL makes a
//! test print why and pass.

use std::{
    path::Path,
    process::Command,
};

use super::{
    programs::lower_c,
    *,
};
use crate::{
    backend::llvm::{
        LlvmOptions,
        OptLevel,
        OutputKind,
        Toolchain,
        emit_module,
    },
    ir::parse_module,
    optimizer::{
        OptimizerOptions,
        optimize,
    },
    test_support::TempDir,
};

/// The types both sides share.
const TYPES: &str = "\
struct I2 { int a, b; };
struct L2 { long long a, b; };
struct LD { long long a; double d; };
struct D2 { double x, y; };
struct F3 { float a, b, c; };
struct F4 { float a, b, c, d; };
struct C3 { char c[3]; };
struct C1 { char c; };
struct S2 { short s; };
struct CD { char c; double d; };
struct IF { int i; float f; };
struct P { char *p; int n; };
struct Big { long long v[4]; };
union U { int i; float f; };
struct Nest { struct I2 in; short s; char c; };
struct X87 { long double x; };
struct Q { __int128 q; };
struct BF { unsigned a : 3, b : 5; int c : 12; };
";

/// The bcc side: functions Clang calls, which compute with the members they
/// receive, and `bcc_calls_clang`, which calls Clang's functions and checks
/// what comes back.
const BCC: &str = "\
struct I2 b_i2(struct I2 x, int k) { struct I2 r; r.a = x.b + k; r.b = x.a - k; return r; }
struct L2 b_l2(struct L2 x, struct L2 y) { struct L2 r; r.a = x.a + y.b; r.b = x.b - y.a; return \
                   r; }
struct LD b_ld(struct LD x, long long k) { x.a += k; x.d /= 2; return x; }
struct D2 b_d2(struct D2 x, struct D2 y) { struct D2 r = { x.x + y.y, x.y * y.x }; return r; }
struct D2 b_d2_many(struct D2 a, struct D2 b, struct D2 c, struct D2 d, struct D2 e) {
  struct D2 r = { a.x + e.x, b.y + c.x + d.y + e.y };
  return r;
}
struct F3 b_f3(struct F3 x) { struct F3 r = { x.c * 2, x.a + x.b, x.b - x.a }; return r; }
struct F4 b_f4(struct F4 x, struct F4 y) {
  struct F4 r = { x.a + y.d, x.b + y.c, x.c + y.b, x.d + y.a };
  return r;
}
struct C3 b_c3(struct C3 x) { struct C3 r; r.c[0] = x.c[2]; r.c[1] = x.c[1]; r.c[2] = x.c[0]; \
                   return r; }
struct C1 b_c1(struct C1 x) { x.c++; return x; }
struct S2 b_s2(struct S2 x) { x.s *= 2; return x; }
struct CD b_cd(struct CD x) { x.c++; x.d *= 3; return x; }
struct IF b_if(struct IF x) { x.i++; x.f += 0.25f; return x; }
struct P b_p(struct P x) { x.p++; x.n--; return x; }
struct Big b_big(struct Big x, int k) { for (int i = 0; i < 4; i++) x.v[i] += k; return x; }
union U b_u(union U x) { x.f = -x.f; return x; }
struct Nest b_nest(struct Nest x) { x.in.a += x.s; x.c++; return x; }
struct X87 b_x87(int k, struct X87 x) { x.x *= k; return x; }
struct Q b_q(struct Q x) { x.q += 1; return x; }
struct BF b_bf(struct BF x) { x.a++; x.b -= 3; x.c = -x.c; return x; }
double b_dsum(struct D2 a, float f, struct F3 b, double d) {
  return a.x + a.y + f + b.a + b.b + b.c + d;
}
long long b_many(long long a, long long b, long long c, long long d, long long e, struct L2 t, \
                   long long f) {
  return a + b + c + d + e + t.a * 100 + t.b * 1000 + f * 10000;
}

struct I2 c_i2(struct I2 x, int k);
struct L2 c_l2(struct L2 x, struct L2 y);
struct D2 c_make_d2(void);
struct D2 c_d2(struct D2 x, struct D2 y);
struct D2 c_d2_many(struct D2 a, struct D2 b, struct D2 c, struct D2 d, struct D2 e);
int c_check_d2(struct D2 x);
struct F3 c_make_f3(void);
struct F3 c_f3(struct F3 x);
int c_check_f3(struct F3 x);
struct LD c_make_ld(void);
struct LD c_ld(struct LD x, long long k);
int c_check_ld(struct LD x);
struct CD c_make_cd(void);
struct CD c_cd(struct CD x);
int c_check_cd(struct CD x);
struct C3 c_c3(struct C3 x);
struct Big c_big(struct Big x, struct Big y);
long long c_many(long long a, long long b, long long c, long long d, long long e, struct L2 t, \
                   long long f);
struct S2 c_s2(struct S2 x);
struct P c_p(struct P x);
union U c_make_u(void);
union U c_u(union U x);
int c_check_u(union U x);
struct IF c_make_if(void);
struct IF c_if(struct IF x);
int c_check_if(struct IF x);
struct X87 c_make_x87(void);
struct X87 c_x87(struct X87 x, int k);
int c_check_x87(struct X87 x);
struct Q c_q(struct Q x);
long long c_vsum(int n, ...);
struct BF c_bf(struct BF x);
double c_dsum(struct D2 a, float f, struct F3 b, double d);

int bcc_calls_clang(void) {
  struct I2 i2 = { 1, 2 };
  struct I2 ri2 = c_i2(i2, 10);
  if (ri2.a != 13 || ri2.b != -9) return 1;
  struct L2 a = { 2, 3 }, b = { 5, 7 };
  struct L2 rl = c_l2(a, b);
  if (rl.a != 10 || rl.b != 21) return 2;
  struct D2 m = c_make_d2();
  if (!c_check_d2(c_d2(m, m))) return 3;
  if (!c_check_d2(c_d2_many(m, m, m, m, c_d2(m, m)))) return 4;
  if (!c_check_f3(c_f3(c_make_f3()))) return 5;
  if (!c_check_ld(c_ld(c_make_ld(), 3))) return 6;
  if (!c_check_cd(c_cd(c_make_cd()))) return 7;
  struct C3 c3 = { { 1, 2, 3 } };
  struct C3 rc3 = c_c3(c3);
  if (rc3.c[0] != 2 || rc3.c[1] != 4 || rc3.c[2] != 6) return 8;
  struct Big x = { { 1, 2, 3, 4 } }, y = { { 10, 20, 30, 40 } };
  struct Big rb = c_big(x, y);
  if (rb.v[0] != 11 || rb.v[3] != 44 || x.v[0] != 1) return 9;
  struct L2 t = { 1, 2 };
  if (c_many(1, 1, 1, 1, 1, t, 3) != 32105) return 10;
  struct S2 s = { 41 };
  if (c_s2(s).s != 42) return 11;
  char text[2] = \"x\";
  struct P p = { text, 21 };
  struct P rp = c_p(p);
  if (rp.p != text || rp.n != 42) return 12;
  if (!c_check_u(c_u(c_make_u()))) return 13;
  if (!c_check_if(c_if(c_make_if()))) return 14;
  if (!c_check_x87(c_x87(c_make_x87(), 2))) return 15;
  struct Q q = { 41 };
  if (c_q(q).q != 42) return 16;
  if (c_vsum(7, 1LL, 2LL, 3LL, 4LL, 5LL, 6LL, 7LL) != 28) return 17;
  if (m.x != 1.25 || m.y != 2.25) return 18;
  struct D2 md = c_d2(m, m);
  if (md.x != 3.5 || md.y != 3.5) return 19;
  struct F3 f3 = c_f3(c_make_f3());
  if (f3.a != 3 || f3.b != 2 || f3.c != 1) return 20;
  struct LD ld = c_ld(c_make_ld(), 3);
  if (ld.a != 15 || ld.d != 1.0) return 21;
  struct CD cd = c_cd(c_make_cd());
  if (cd.c != 8 || cd.d != 2.5) return 22;
  if (c_u(c_make_u()).f != 1.5f) return 23;
  struct IF iff = c_if(c_make_if());
  if (iff.i != 6 || iff.f != 1.0f) return 24;
  if (c_x87(c_make_x87(), 2).x != 1.5L) return 25;
  struct BF bf = { 1, 2, -3 };
  struct BF rbf = c_bf(bf);
  if (rbf.a != 3 || rbf.b != 4 || rbf.c != -1003) return 26;
  if (c_dsum(md, 0.5f, f3, 0.25) != 13.75) return 27;
  return 0;
}
";

/// The Clang side: `main` calls bcc's functions and then `bcc_calls_clang`.
const CLANG: &str = "\
#include <stdarg.h>

struct I2 b_i2(struct I2 x, int k);
struct L2 b_l2(struct L2 x, struct L2 y);
struct LD b_ld(struct LD x, long long k);
struct D2 b_d2(struct D2 x, struct D2 y);
struct D2 b_d2_many(struct D2 a, struct D2 b, struct D2 c, struct D2 d, struct D2 e);
struct F3 b_f3(struct F3 x);
struct F4 b_f4(struct F4 x, struct F4 y);
struct C3 b_c3(struct C3 x);
struct C1 b_c1(struct C1 x);
struct S2 b_s2(struct S2 x);
struct CD b_cd(struct CD x);
struct IF b_if(struct IF x);
struct P b_p(struct P x);
struct Big b_big(struct Big x, int k);
union U b_u(union U x);
struct Nest b_nest(struct Nest x);
struct X87 b_x87(int k, struct X87 x);
struct Q b_q(struct Q x);
struct BF b_bf(struct BF x);
double b_dsum(struct D2 a, float f, struct F3 b, double d);
long long b_many(long long a, long long b, long long c, long long d, long long e, struct L2 t,
                 long long f);
int bcc_calls_clang(void);

struct I2 c_i2(struct I2 x, int k) { struct I2 r = { x.a + k + 2, x.b - k - 1 }; return r; }
struct L2 c_l2(struct L2 x, struct L2 y) { struct L2 r = { x.a * y.a, x.b * y.b }; return r; }
struct D2 c_make_d2(void) { struct D2 r = { 1.25, 2.25 }; return r; }
struct D2 c_d2(struct D2 x, struct D2 y) { struct D2 r = { x.x + y.y, x.y + y.x }; return r; }
struct D2 c_d2_many(struct D2 a, struct D2 b, struct D2 c, struct D2 d, struct D2 e) { return e; }
int c_check_d2(struct D2 x) { return x.x == 3.5 && x.y == 3.5; }
struct F3 c_make_f3(void) { struct F3 r = { 1, 2, 3 }; return r; }
struct F3 c_f3(struct F3 x) { struct F3 r = { x.c, x.b, x.a }; return r; }
int c_check_f3(struct F3 x) { return x.a == 3 && x.b == 2 && x.c == 1; }
struct LD c_make_ld(void) { struct LD r = { 5, 0.5 }; return r; }
struct LD c_ld(struct LD x, long long k) { x.a *= k; x.d *= 2; return x; }
int c_check_ld(struct LD x) { return x.a == 15 && x.d == 1.0; }
struct CD c_make_cd(void) { struct CD r = { 7, 1.5 }; return r; }
struct CD c_cd(struct CD x) { x.c++; x.d += 1; return x; }
int c_check_cd(struct CD x) { return x.c == 8 && x.d == 2.5; }
struct C3 c_c3(struct C3 x) { for (int i = 0; i < 3; i++) x.c[i] *= 2; return x; }
struct Big c_big(struct Big x, struct Big y) {
  for (int i = 0; i < 4; i++) x.v[i] += y.v[i];
  return x;
}
long long c_many(long long a, long long b, long long c, long long d, long long e, struct L2 t,
                 long long f) {
  return a + b + c + d + e + t.a * 100 + t.b * 1000 + f * 10000;
}
struct S2 c_s2(struct S2 x) { x.s += 1; return x; }
struct P c_p(struct P x) { x.n *= 2; return x; }
union U c_make_u(void) { union U u; u.f = 1.5f; return u; }
union U c_u(union U x) { return x; }
int c_check_u(union U x) { return x.f == 1.5f; }
struct IF c_make_if(void) { struct IF r = { 3, 0.5f }; return r; }
struct IF c_if(struct IF x) { x.i *= 2; x.f *= 2; return x; }
int c_check_if(struct IF x) { return x.i == 6 && x.f == 1.0f; }
struct X87 c_make_x87(void) { struct X87 r = { 0.75L }; return r; }
struct X87 c_x87(struct X87 x, int k) { x.x *= k; return x; }
int c_check_x87(struct X87 x) { return x.x == 1.5L; }
struct Q c_q(struct Q x) { x.q += 1; return x; }
struct BF c_bf(struct BF x) { x.a += 2; x.b *= 2; x.c -= 1000; return x; }
double c_dsum(struct D2 a, float f, struct F3 b, double d) {
  return a.x + a.y + f + b.a + b.b + b.c + d;
}
long long c_vsum(int n, ...) {
  va_list list;
  va_start(list, n);
  long long sum = 0;
  for (int i = 0; i < n; i++) sum += va_arg(list, long long);
  va_end(list);
  return sum;
}

int main(void) {
  struct I2 i2 = { 5, 7 };
  struct I2 r = b_i2(i2, 3);
  if (r.a != 10 || r.b != 2) return 10;
  struct L2 l1 = { 1, 2 }, l2 = { 30, 40 };
  struct L2 rl = b_l2(l1, l2);
  if (rl.a != 41 || rl.b != -28) return 11;
  struct LD ld = { 7, 2.5 };
  struct LD rld = b_ld(ld, 3);
  if (rld.a != 10 || rld.d != 1.25) return 12;
  struct D2 d1 = { 1.5, 2.5 }, d2 = { 3.5, 4.5 }, d3 = { 5.5, 6.5 }, d4 = { 7.5, 8.5 },
            d5 = { 9.5, 10.5 };
  struct D2 rd = b_d2(d1, d2);
  if (rd.x != 6.0 || rd.y != 8.75) return 13;
  rd = b_d2_many(d1, d2, d3, d4, d5);
  if (rd.x != 11.0 || rd.y != 29.0) return 14;
  struct F3 f3 = { 1, 2, 3 };
  struct F3 rf3 = b_f3(f3);
  if (rf3.a != 6 || rf3.b != 3 || rf3.c != 1) return 15;
  struct F4 f4a = { 1, 2, 3, 4 }, f4b = { 5, 6, 7, 8 };
  struct F4 rf4 = b_f4(f4a, f4b);
  if (rf4.a != 9 || rf4.b != 9 || rf4.c != 9 || rf4.d != 9) return 16;
  struct C3 c3 = { { 1, 2, 3 } };
  struct C3 rc3 = b_c3(c3);
  if (rc3.c[0] != 3 || rc3.c[1] != 2 || rc3.c[2] != 1) return 17;
  struct C1 c1 = { 41 };
  if (b_c1(c1).c != 42) return 18;
  struct S2 s2 = { 21 };
  if (b_s2(s2).s != 42) return 19;
  struct CD cd = { 1, 2.5 };
  struct CD rcd = b_cd(cd);
  if (rcd.c != 2 || rcd.d != 7.5) return 20;
  struct IF iff = { 1, 2.5f };
  struct IF rif = b_if(iff);
  if (rif.i != 2 || rif.f != 2.75f) return 21;
  char text[4] = \"abc\";
  struct P p = { text, 3 };
  struct P rp = b_p(p);
  if (rp.p != text + 1 || rp.n != 2) return 22;
  struct Big big = { { 1, 2, 3, 4 } };
  struct Big rb = b_big(big, 10);
  if (rb.v[0] != 11 || rb.v[3] != 14 || big.v[0] != 1) return 23;
  union U u;
  u.f = 2.5f;
  union U ru = b_u(u);
  if (ru.f != -2.5f) return 24;
  struct Nest n = { { 1, 2 }, 3, 4 };
  struct Nest rn = b_nest(n);
  if (rn.in.a != 4 || rn.in.b != 2 || rn.s != 3 || rn.c != 5) return 25;
  struct L2 t = { 1, 2 };
  if (b_many(1, 1, 1, 1, 1, t, 3) != 32105) return 26;
  struct X87 x87 = { 2.5L };
  if (b_x87(3, x87).x != 7.5L) return 27;
  struct Q q = { 41 };
  if (b_q(q).q != 42) return 28;
  struct BF bf = { 6, 20, 50 };
  struct BF rbf = b_bf(bf);
  if (rbf.a != 7 || rbf.b != 17 || rbf.c != -50) return 29;
  if (b_dsum(d1, 0.5f, f3, 0.25) != 10.75) return 30;
  int status = bcc_calls_clang();
  return status ? 100 + status : 0;
}
";

/// A freestanding start for System V executables, which link without a C
/// library: `_start` calls `main` and exits with its status, and the memory
/// functions LLVM may call are written with volatile accesses so that they
/// do not become calls to themselves.
const FREESTANDING: &str = "
int main(void);
__attribute__((force_align_arg_pointer, noreturn)) void _start(void) {
  long status = main();
  __asm__ volatile(\"syscall\" :: \"a\"(60), \"D\"(status) : \"rcx\", \"r11\", \"memory\");
  __builtin_unreachable();
}
void *memcpy(void *to, const void *from, unsigned long n) {
  volatile char *t = to;
  const volatile char *f = from;
  while (n--) *t++ = *f++;
  return to;
}
void *memmove(void *to, const void *from, unsigned long n) {
  volatile char *t = to;
  const volatile char *f = from;
  if (t < f) { while (n--) *t++ = *f++; } else { while (n--) t[n] = f[n]; }
  return to;
}
void *memset(void *to, int byte, unsigned long n) {
  volatile char *t = to;
  while (n--) *t++ = (char)byte;
  return to;
}
";

#[test]
fn structures_cross_between_bcc_and_clang() {
    for target in [Target::WindowsGnu, Target::LinuxGnu] {
        let source = format!("{TYPES}{BCC}");
        let ll = |arena: &Bump| -> String {
            let tu = Bump::new();
            let mut module = lower_c(&tu, arena, &source, target);
            let scratch = Bump::new();
            let mut options = OptimizerOptions::default();
            options.verify_each = true;
            _ = optimize(&mut module, &options, &scratch);
            lower_abi(&mut module, target, &scratch).unwrap();
            let mut text = String::new();
            emit_module(&module, target, &mut text).unwrap();
            text
        };
        let arena = Bump::new();
        assert_conforms(
            "structures",
            target,
            &ll(&arena),
            &format!("{TYPES}{CLANG}"),
        );
    }
}

/// A bcc variadic function, written as IR since lowering does not lower
/// `va_arg` yet, reads groups of a `long long`, a `double`, a pointer to an
/// `int` and an `int` from Clang's call: more than either register class
/// holds on System V.
#[test]
fn variadic_arguments_cross_from_clang() {
    let ir = "\
function @b_vmix(i32, ...) -> i64 external {
    slot0 = stack_slot 24, align 8
block0(v0: i32):
    v1 = stack_addr slot0
    va_start v1
    v2 = iconst.i32 0
    v3 = iconst.i64 0
    jump block1(v2, v3)
block1(v4: i32, v5: i64):
    v6 = icmp.i32 slt v4, v0
    brif v6, block2, block3
block2:
    v7 = va_arg.i64 v1
    v8 = va_arg.f64 v1
    v9 = va_arg.ptr v1
    v10 = va_arg.i32 v1
    v11 = fptosi.i64 v8
    v12 = load.i32 v9, align 4
    v13 = sext.i64 v12
    v14 = sext.i64 v10
    v15 = iadd.i64 v5, v7
    v16 = iadd.i64 v15, v11
    v17 = iadd.i64 v16, v13
    v18 = iadd.i64 v17, v14
    v19 = iconst.i32 1
    v20 = iadd.i32 v4, v19
    jump block1(v20, v18)
block3:
    va_end v1
    return v5
}
";
    let clang = "\
long long b_vmix(int n, ...);
int main(void) {
  int a = 1000, b = 20000;
  long long sum = b_vmix(4, 1LL, 2.0, &a, 3, 10LL, 20.0, &b, 30, 100LL, 200.0, &a, 300, 1000LL,
                         2000.0, &b, 3000);
  return sum == 3333 + 3333 + 2000 + 40000 ? 0 : 1;
}
";
    for target in [Target::WindowsGnu, Target::LinuxGnu] {
        let arena = Bump::new();
        let mut module = parse_module(&arena, ir).unwrap();
        let scratch = Bump::new();
        lower_abi(&mut module, target, &scratch).unwrap();
        assert_verified(&module, Profile::PostAbi, &module.to_string());
        let mut text = String::new();
        emit_module(&module, target, &mut text).unwrap();
        assert_conforms("varargs", target, &text, clang);
    }
}

/// Builds the bcc LLVM IR `ll` and the Clang C source `c` into one
/// executable for `target` at `-O0` and `-O2`, runs each, and asserts that
/// it exits with 0.
fn assert_conforms(name: &str, target: Target, ll: &str, c: &str) {
    let Some(runner) = Runner::for_target(target) else {
        return;
    };
    let toolchain = match Toolchain::bundled(target) {
        | Ok(toolchain) => toolchain,
        | Err(error) => {
            eprintln!("skipping {name} for {target:?}: {error}");
            return;
        },
    };
    let directory = TempDir::new("abi-conformance");
    let ll_path = directory.join(format!("{name}.ll"));
    std::fs::write(&ll_path, ll).unwrap();
    let c_path = directory.join(format!("{name}-clang.c"));
    let c = match runner {
        | Runner::Native => c.to_string(),
        | Runner::Wsl => format!("{c}{FREESTANDING}"),
    };
    std::fs::write(&c_path, &c).unwrap();
    for opt in [OptLevel::O0, OptLevel::O2] {
        let executable = directory.join(format!("{name}-{opt:?}.exe"));
        let options = LlvmOptions {
            opt: Some(opt),
            disable_llvm_passes: false,
            target,
            kind: OutputKind::Executable,
        };
        let mut command = toolchain.command(&ll_path, &executable, options);
        _ = command.args(["-x", "c"]).arg(&c_path);
        if runner == Runner::Wsl {
            _ = command.args([
                "-nostdlib",
                "-static",
                "-ffreestanding",
                "-fno-stack-protector",
            ]);
        }
        let output = command.output().unwrap();
        assert!(
            output.status.success(),
            "{name} for {target:?} at {opt:?} does not build:\n{}\n{ll}",
            String::from_utf8_lossy(&output.stderr)
        );
        let status = runner.run(&executable);
        assert_eq!(
            status,
            Some(0),
            "{name} for {target:?} at {opt:?} failed its check {status:?}:\n{ll}"
        );
    }
}

/// How a test runs an executable for its target.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Runner {
    /// On the host.
    Native,
    /// A static Linux executable under the Windows Subsystem for Linux.
    Wsl,
}

impl Runner {
    /// The runner for `target` on this host, or `None` after explaining why
    /// the test cannot run.
    fn for_target(target: Target) -> Option<Self> {
        let native = match target {
            | Target::WindowsGnu => cfg!(all(windows, target_env = "gnu")),
            | Target::WindowsMsvc => cfg!(all(windows, target_env = "msvc")),
            | Target::LinuxGnu => cfg!(all(target_os = "linux", target_env = "gnu")),
            | Target::LinuxMusl => cfg!(all(target_os = "linux", target_env = "musl")),
        };
        if native {
            return Some(Self::Native);
        }
        if target == Target::LinuxGnu
            && cfg!(windows)
            && Command::new("wsl.exe")
                .args(["-e", "true"])
                .output()
                .is_ok_and(|output| output.status.success())
        {
            return Some(Self::Wsl);
        }
        eprintln!("skipping {target:?}: this host cannot run its executables");
        None
    }

    /// Runs `executable`, returning its exit status.
    fn run(self, executable: &Path) -> Option<i32> {
        let output = match self {
            | Self::Native => Command::new(executable).output(),
            | Self::Wsl => Command::new("wsl.exe")
                .arg("-e")
                .arg(format!(
                    "./{}",
                    executable.file_name().unwrap().to_str().unwrap()
                ))
                .current_dir(executable.parent().unwrap())
                .output(),
        };
        output.unwrap().status.code()
    }
}
