//! C programs that pass and return structures and unions by value, lowered
//! for each target and run in the interpreter before and after ABI
//! lowering; both runs must return the expected status. The end-to-end tests
//! in `pipeline/tests/middle_end.rs` also build them through each LLVM
//! comparison arm and run them natively.

use std::path::Path;

use super::*;
use crate::{
    backend::interpreter::{
        DefaultHost,
        Outcome,
        RuntimeValue,
        Termination,
        run,
    },
    configuration::CompilerConfiguration,
    headers::HeaderSearch,
    lowering::{
        lower_translation_unit,
        tests::programs::Program,
    },
    optimizer::{
        OptimizerOptions,
        optimize,
    },
    pipeline::{
        analyze_translation_unit,
        parse_translation_unit,
    },
    translation_phases::Context,
};

/// The programs, from structures of integers through SSE-class ones of
/// `float` and `double`, mixed eightbytes, unions, bit-fields and
/// structures too large for registers.
pub(crate) const PROGRAMS: &[Program] = &[
    Program {
        name: "aggregate_varargs",
        source: "struct S { int x; }; struct M { int i; double d; }; struct D { double a,b; }; struct B { int a[6]; }; union U { int i; double d; };
static int take(int n, ...) { __builtin_va_list ap, cp; __builtin_va_start(ap,n);
 for(int i=0;i<n;i++) { (void)__builtin_va_arg(ap,long long); (void)__builtin_va_arg(ap,double); }
 __builtin_va_copy(cp,ap);
 struct S s=__builtin_va_arg(ap,struct S); struct M m=__builtin_va_arg(ap,struct M); struct D d=__builtin_va_arg(ap,struct D); struct B b=__builtin_va_arg(ap,struct B); union U u=__builtin_va_arg(ap,union U); int end=__builtin_va_arg(ap,int);
 struct S c=__builtin_va_arg(cp,struct S); __builtin_va_end(cp); __builtin_va_end(ap);
 return s.x!=3 || c.x!=3 || m.i!=4 || m.d!=5.5 || d.a!=6.5 || d.b!=7.5 || b.a[0]!=8 || b.a[5]!=13 || u.d!=14.5 || end!=99;
}
int main(void) { struct S s={3}; struct M m={4,5.5}; struct D d={6.5,7.5}; struct B b={{8,9,10,11,12,13}}; union U u; u.d=14.5;
 int (*p)(int,...)=take; return take(0,s,m,d,b,u,99) || p(8,1LL,1.,2LL,2.,3LL,3.,4LL,4.,5LL,5.,6LL,6.,7LL,7.,8LL,8.,s,m,d,b,u,99); }",
        expected: 0,
    },
    Program {
        name: "aggregate_argument_snapshot",
        source: "struct S { int x; }; static struct S s; static int evaluations;
static int mutate(void) { s.x = 2; return 0; }
static struct S assign(void) { ++evaluations; return s = (struct S){1}; }
static int take(struct S a, int b) { a.x += b; return a.x; }
int main(void) {
    int direct = take((s = (struct S){1}), mutate());
    int (*indirect)(struct S, int) = take;
    int via_pointer = indirect(assign(), mutate());
    return direct == 1 && via_pointer == 1 && evaluations == 1 ? 0 : 1;
}",
        expected: 0,
    },
    Program {
        name:     "pair_in_registers",
        source:   "struct pair { long a, b; };
static struct pair swap(struct pair p) { struct pair r; r.a = p.b; r.b = p.a; return r; }
int main(void) { struct pair p = { 3, 40 }; struct pair q = swap(p); return q.a + q.b * 0 + \
                   swap(q).b; }",
        expected: 80,
    },
    Program {
        name:     "member_of_a_returned_structure",
        source:   "struct s { int x, y, z; };
static struct s make(int k) { struct s r = { k, k * 2, k * 3 }; return r; }
int main(void) { return make(4).z + make(5).x; }",
        expected: 17,
    },
    Program {
        name:     "arrays_and_chars_in_two_eightbytes",
        source:   "struct mix { int a[3]; char c; };
static int total(struct mix m) { return m.a[0] + m.a[1] + m.a[2] + m.c; }
static struct mix bump(struct mix m) { m.a[1] += 10; m.c++; return m; }
int main(void) { struct mix m = { { 1, 2, 3 }, 4 }; return total(bump(bump(m))) + total(m); }",
        expected: 42,
    },
    Program {
        name:     "large_structures_in_memory",
        source:   "struct big { long v[5]; };
static struct big scale(struct big b, int k) { for (int i = 0; i < 5; i++) b.v[i] *= k; return b; }
int main(void) {
  struct big b = { { 1, 2, 3, 4, 5 } };
  struct big c = scale(b, 3);
  return c.v[0] + c.v[4] + b.v[4];
}",
        expected: 23,
    },
    Program {
        name:     "three_bytes",
        source:   "struct rgb { unsigned char r, g, b; };
static struct rgb mix(struct rgb x, struct rgb y) {
  struct rgb m = { (x.r + y.r) / 2, (x.g + y.g) / 2, (x.b + y.b) / 2 };
  return m;
}
int main(void) { struct rgb a = { 10, 20, 30 }, b = { 30, 40, 50 }; struct rgb c = mix(a, b); \
                   return c.r + c.g + c.b; }",
        expected: 90,
    },
    Program {
        name:     "unions_by_value",
        source:   "union u { int i; char c[4]; };
static union u flip(union u v) { char t = v.c[0]; v.c[0] = v.c[1]; v.c[1] = t; return v; }
int main(void) { union u v; v.i = 0; v.c[0] = 1; v.c[1] = 2; v = flip(v); return v.c[0] * 10 + \
                   v.c[1]; }",
        expected: 21,
    },
    Program {
        name:     "recursion_through_results",
        source:   "struct fib { long a, b; };
static struct fib step(int n) {
  if (n == 0) { struct fib f = { 0, 1 }; return f; }
  struct fib p = step(n - 1);
  struct fib r = { p.b, p.a + p.b };
  return r;
}
int main(void) { return step(10).a; }",
        expected: 55,
    },
    Program {
        name:     "function_pointers_and_self_assignment",
        source:   "struct p { int x, y; };
static struct p add(struct p a, struct p b) { struct p r = { a.x + b.x, a.y + b.y }; return r; }
int main(void) {
  struct p (*f)(struct p, struct p) = add;
  struct p s = { 1, 2 };
  s = f(s, s);
  s = add(s, f(s, s));
  return s.x * 10 + s.y;
}",
        expected: 72,
    },
    Program {
        name:     "registers_run_out",
        source:   "struct two { long a, b; };
static long many(long a, long b, long c, long d, long e, struct two t, long f, struct two u) {
  return a + b + c + d + e + t.a * 10 + t.b * 20 + f + u.a * 3 + u.b;
}
int main(void) { struct two t = { 1, 2 }; struct two u = { 4, 5 }; return many(1, 1, 1, 1, 1, t, \
                   6, u); }",
        expected: 78,
    },
    Program {
        name:     "pointers_inside_structures",
        source:   "struct view { const char *text; int length; };
static struct view tail(struct view v) { v.text++; v.length--; return v; }
int main(void) { struct view v = { \"hello\", 5 }; struct view t = tail(tail(v)); return t.text[0] \
                   + t.length; }",
        expected: 111,
    },
    Program {
        name:     "unprototyped_callee",
        source:   "struct p { int x, y; };
int dot();
int main(void) { struct p a = { 2, 3 }, b = { 4, 5 }; return dot(a, b); }
int dot(struct p a, struct p b) { return a.x * b.x + a.y * b.y; }",
        expected: 23,
    },
    Program {
        name:     "doubles_in_sse_registers",
        source:   "struct vec { double x, y; };
static struct vec add(struct vec a, struct vec b) { struct vec r = { a.x + b.x, a.y + b.y };                    return r; }
static struct vec scale(struct vec v, double k) { v.x *= k; v.y *= k; return v; }
static double dot(struct vec a, struct vec b) { return a.x * b.x + a.y * b.y; }
int main(void) {
  struct vec a = { 1.5, 2.0 }, b = { 0.5, -1.0 };
  struct vec s = scale(add(a, b), 4.0);
  return (int)dot(s, a) + (int)s.y;
}",
        expected: 24,
    },
    Program {
        name:     "floats_packed_in_registers",
        source:   "struct f3 { float a, b, c; };
static struct f3 rotate(struct f3 v) { struct f3 r = { v.b, v.c, v.a }; return r; }
static float weigh(struct f3 v) { return v.a + 10 * v.b + 100 * v.c; }
int main(void) { struct f3 v = { 1.0f, 2.0f, 3.0f }; return (int)weigh(rotate(v)) -                    (int)weigh(v) + 200; }",
        expected: 11,
    },
    Program {
        name:     "integer_and_sse_eightbytes",
        source:   "struct dl { double d; long long n; };
struct if2 { int i; float f; };
static struct dl mix(struct dl x, struct if2 y) { x.d += y.f; x.n -= y.i; return x; }
static struct if2 halve(struct if2 y) { y.i /= 2; y.f /= 2; return y; }
int main(void) {
  struct dl x = { 0.25, 100 };
  struct if2 y = { 30, 7.5f };
  struct dl r = mix(x, halve(y));
  return (int)(r.d * 10) + (int)r.n;
}",
        expected: 125,
    },
    Program {
        name:     "float_then_double",
        source:   "struct fd { float f; double d; };
struct f1 { float f; };
static struct f1 shrink(struct fd v) { struct f1 r = { v.f + (float)v.d }; return r; }
int main(void) { struct fd v = { 1.5f, 2.25 }; return (int)(shrink(v).f * 4); }",
        expected: 15,
    },
    Program {
        name:     "sse_registers_run_out",
        source:   "struct d2 { double x, y; };
static double last(struct d2 a, struct d2 b, struct d2 c, struct d2 d, double e, struct d2 f,                    int k) {
  return a.x + b.y + c.x + d.y + e + f.x * k + f.y;
}
int main(void) {
  struct d2 a = { 1, 2 }, b = { 3, 4 }, c = { 5, 6 }, d = { 7, 8 }, f = { 9, 10 };
  return (int)(last(a, b, c, d, 0.5, f, 2) * 2);
}",
        expected: 93,
    },
    Program {
        name:     "doubles_in_memory",
        source:   "struct d3 { double x, y, z; };
static struct d3 cross(struct d3 a, struct d3 b) {
  struct d3 r = { a.y * b.z - a.z * b.y, a.z * b.x - a.x * b.z, a.x * b.y - a.y * b.x };
  return r;
}
int main(void) {
  struct d3 a = { 1, 2, 3 }, b = { 4, 5, 6 };
  struct d3 c = cross(a, b);
  return (int)(c.x * 100 + c.y * 10 + c.z) + 400;
}",
        expected: 157,
    },
    Program {
        name:     "union_of_float_and_integer",
        source:   "union fu { float f; unsigned u; };
static union fu negate(union fu v) { v.u ^= 0x80000000u; return v; }
int main(void) { union fu v; v.f = 2.5f; v = negate(v); return (int)(v.f * -4); }",
        expected: 10,
    },
    Program {
        name:     "bit_fields_by_value",
        source:   "struct flags { unsigned a : 3, b : 5; int c : 12; };
static struct flags bump(struct flags f) { f.a++; f.b += 2; f.c -= 100; return f; }
int main(void) { struct flags f = { 6, 20, 50 }; f = bump(f); return f.a + f.b + (f.c + 50); }",
        expected: 29,
    },
];

/// The targets every program runs for: System V, and Win64 with either C
/// library, whose `long` widths differ.
const TARGETS: [Target; 3] = [Target::LinuxGnu, Target::WindowsGnu, Target::WindowsMsvc];

#[test]
fn programs_return_the_same_before_and_after_abi_lowering() {
    for program in PROGRAMS {
        for target in TARGETS {
            let tu = Bump::new();
            let arena = Bump::new();
            let mut module = lower_c(&tu, &arena, program.source, target);
            let before = module.to_string();
            assert_eq!(
                main_status(&module),
                program.expected,
                "{} before ABI lowering for {target:?}:\n{before}",
                program.name
            );
            let scratch = Bump::new();
            lower_abi(&mut module, target, &scratch).unwrap();
            assert_verified(&module, Profile::PostAbi, &module.to_string());
            assert_eq!(
                main_status(&module),
                program.expected,
                "{} after ABI lowering for {target:?}:\n{module}",
                program.name
            );
            // The driver's order: the optimizer, then ABI lowering.
            let arena = Bump::new();
            let mut module = lower_c(&tu, &arena, program.source, target);
            let mut options = OptimizerOptions::default();
            options.verify_each = true;
            _ = optimize(&mut module, &options, &scratch);
            lower_abi(&mut module, target, &scratch).unwrap();
            assert_eq!(
                main_status(&module),
                program.expected,
                "{} optimized, then ABI lowered for {target:?}:\n{module}",
                program.name
            );
        }
    }
}

/// Runs phases 1-7 and lowering on `source` for `target`, returning the
/// verified pre-ABI module.
pub(super) fn lower_c<'ir>(
    tu: &Bump,
    arena: &'ir Bump,
    source: &str,
    target: Target,
) -> Module<'ir> {
    let configuration = CompilerConfiguration::default().with_target(target);
    let source = tu.alloc_str(&format!("{source}\n"));
    let mut context = Context::with_configuration(tu, configuration);
    let unit = parse_translation_unit(
        &mut context,
        Path::new("<test>"),
        source,
        HeaderSearch::default(),
    );
    let sema = analyze_translation_unit(&mut context, &unit);
    let mut scratch = Bump::new();
    let module = lower_translation_unit(&context, &unit, &sema, target, arena, &mut scratch)
        .unwrap_or_else(|errors| {
            let errors: Vec<String> = errors.errors.iter().map(ToString::to_string).collect();
            panic!("lowering failed: {errors:?}\n{source}")
        });
    assert_verified(&module, Profile::PreAbi, &module.to_string());
    module
}

/// The status `main` of `module` returns in the interpreter.
fn main_status(module: &Module<'_>) -> i32 {
    let arena = Bump::new();
    let mut host = DefaultHost::new_in(&arena);
    match run(module, "main", &[], &mut host) {
        | Ok(Outcome {
            termination: Termination::Returned(Some(RuntimeValue::Int(bits))),
            ..
        }) => i32::from_le_bytes(bits.to_le_bytes()[..4].try_into().unwrap()),
        | other => panic!(
            "{:?}",
            other.map_err(|trap| trap.display_in(module).to_string())
        ),
    }
}

#[test]
fn review_wide_scalars_execute_before_and_after_abi_lowering() {
    let source = "__int128 f(__int128 x){return x+1;} long long low(__int128 x){return (long \
                  long)x;} int main(void){ __int128 x=((__int128)0x1122334455667788ULL<<64)+42; \
                  __int128 (*p)(__int128)=f; return low(42)!=42 || f(x)!=x+1 || p(x)!=x+1; }";
    for target in [Target::WindowsGnu, Target::WindowsMsvc, Target::LinuxGnu] {
        let arena = Bump::new();
        let tu = Bump::new();
        let mut module = lower_c(&tu, &arena, source, target);
        assert_eq!(main_status(&module), 0);
        lower_abi(&mut module, target, &Bump::new()).unwrap();
        assert_eq!(main_status(&module), 0);
    }
}

#[test]
fn review_quad_aggregate_preserves_all_sixteen_bytes() {
    let source = "struct S { __float128 x; }; struct S s; int f(struct S x){return 0;} struct S \
                  echo(struct S x){return x;} int main(void){ if(f(s)) return 1; struct S a; \
                  unsigned char *p=(unsigned char *)&a; for(int i=0;i<16;i++)p[i]=i+1; struct S \
                  b=echo(a); unsigned char *q=(unsigned char *)&b; for(int \
                  i=0;i<16;i++)if(q[i]!=i+1)return 2; return 0; }";
    for target in [Target::LinuxGnu, Target::WindowsGnu] {
        let arena = Bump::new();
        let tu = Bump::new();
        let mut module = lower_c(&tu, &arena, source, target);
        lower_abi(&mut module, target, &Bump::new()).unwrap();
        assert_eq!(main_status(&module), 0);
    }
}

#[test]
fn review_overlapping_quad_members_pack_a_full_vector() {
    let source = "union U { __float128 x; float f; }; union U echo(union U x){return x;} int \
                  main(void){ union U a; unsigned char *p=(unsigned char *)&a; for(int \
                  i=0;i<16;i++)p[i]=i+1; union U b=echo(a); unsigned char *q=(unsigned char *)&b; \
                  for(int i=0;i<16;i++)if(q[i]!=i+1)return 1; return 0; }";
    let arena = Bump::new();
    let tu = Bump::new();
    let mut module = lower_c(&tu, &arena, source, Target::LinuxGnu);
    lower_abi(&mut module, Target::LinuxGnu, &Bump::new()).unwrap();
    assert_eq!(main_status(&module), 0);
}
