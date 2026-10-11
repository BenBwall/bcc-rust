//! C programs that pass and return structures and unions by value, lowered
//! for each target and run in the interpreter before and after ABI
//! lowering; both runs must return the expected status.

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
    lowering::lower_translation_unit,
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

/// A C program and the status its `main` returns.
pub(super) struct Program {
    pub(super) name:     &'static str,
    pub(super) source:   &'static str,
    pub(super) expected: u8,
}

pub(super) const PROGRAMS: &[Program] = &[
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
fn main_status(module: &Module<'_>) -> u8 {
    let arena = Bump::new();
    let mut host = DefaultHost::new_in(&arena);
    match run(module, "main", &[], &mut host) {
        | Ok(Outcome {
            termination: Termination::Returned(Some(RuntimeValue::Int(bits))),
            ..
        }) => u8::try_from(bits & 0xFF).unwrap(),
        | other => panic!(
            "{:?}",
            other.map_err(|trap| trap.display_in(module).to_string())
        ),
    }
}
