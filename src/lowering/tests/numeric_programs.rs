//! Programs that exercise floating point and bit-fields, run in the IR
//! interpreter: each lowers on every target, verifies, and returns the
//! status recorded here.
//!
//! The interpreter has no `f80`, so a program using `long double` runs on
//! the MSVC target, where `long double` is `f64`; the others run on every
//! target and must agree.

use super::{
    programs::Program,
    *,
};
use crate::backend::{
    interpreter::{
        Outcome,
        RuntimeValue,
        Termination,
        run_text,
    },
    llvm::{
        LlvmOptions,
        OptLevel,
        OutputKind,
        Toolchain,
        compile_module,
    },
};

const TARGETS: [Target; 4] = [
    Target::LinuxGnu,
    Target::LinuxMusl,
    Target::WindowsGnu,
    Target::WindowsMsvc,
];

pub(super) const FLOAT_PROGRAMS: &[Program] = &[
    Program {
        name:     "float_horner_polynomial",
        source:   "double horner(double x) { return ((2.0 * x + 3.0) * x + 4.0) * x + 5.0; }
int main(void) { return (int)horner(2.0); }",
        expected: 41,
    },
    Program {
        name:     "float_geometric_series",
        source:   "int main(void) {
  double s = 0;
  for (int i = 1; i <= 10; i++) s += 1.0 / (1 << i);
  return (int)(s * 256);
}",
        expected: 255,
    },
    Program {
        name:     "float_newton_square_root",
        source:   "static double root(double x) {
  double g = x / 2;
  for (int i = 0; i < 40; i++) g = (g + x / g) / 2;
  return g;
}
int main(void) { return (int)(root(144.0) + 0.5) + (int)(root(2.0) * root(2.0) + 0.5); }",
        expected: 14,
    },
    Program {
        name:     "float_conversions_truncate_and_round_trip",
        source:   "int main(void) {
  float f = 2.75f;
  double d = f;
  long long l = (long long)(d * 4);
  unsigned u = (unsigned)3.99;
  int i = (int)-7.9;
  _Bool b = 0.25;
  _Bool z = 0.0;
  unsigned long long big = 1ull << 63;
  double wide = big;
  return (int)(f * 2) + (int)l + (int)u + i + b + z + ((unsigned long long)wide == big) + 50;
}",
        expected: 64,
    },
    Program {
        name:     "float_comparisons_and_nan",
        source:   "static double zero(void) { return 0.0; }
int main(void) {
  double nan = zero() / zero();
  int r = 0;
  r += nan == nan;
  r += nan != nan;
  r += (nan < 1.0) + (nan > 1.0) + (nan <= nan) + (nan >= nan);
  r += !nan ? 0 : 10;
  if (nan) r += 100;
  r += (1.5 < 2.5) + (2.5 <= 2.5) + (3.0 >= 4.0) * 50;
  r += !zero() * 4;
  return r;
}",
        expected: 117,
    },
    Program {
        name:     "float_recursion_and_float_parameters",
        source:   "static double power(double b, int n) { return n == 0 ? 1.0 : b * power(b, n - \
                   1); }
static float average(float a, float b) { return (a + b) / 2; }
int main(void) { return (int)power(2.0, 7) + (int)average(3.0f, 4.0f); }",
        expected: 131,
    },
    Program {
        name:     "float_compound_assignment_and_increment",
        source:   "int main(void) {
  int i = 10;
  i *= 1.5;
  i += 0.9;
  double d = 1;
  d /= 4;
  d++;
  float f = 3;
  f -= 0.5f;
  --f;
  i -= (int)(d * 4 + f * 2);
  return i + (int)(f * 10);
}",
        expected: 22,
    },
    Program {
        name:     "float_static_initializers",
        source:   "static double table[4] = { 0.5, 1.5, 2, -3.25 };
static float scale = 2.5f;
double total = 7;
int truncated = 9.75;
_Bool flag = 0.5;
int main(void) {
  double s = 0;
  for (int i = 0; i < 4; i++) s += table[i];
  return (int)((s + total) * scale) + truncated + flag;
}",
        expected: 29,
    },
    Program {
        name:     "float_conditional_and_logical_operators",
        source:   "static int classify(double x) {
  if (x < 0) return 1;
  if (x && x < 1) return 2;
  return x ? 3 : 4;
}
int main(void) {
  double third = 1.0 / 3;
  return classify(-2.5) + classify(0.5) * 10 + classify(0) * 3 + (third && 0.0) +
         (third || 0.0) * 8 + (int)(third > 0 ? 2.5 : 1);
}",
        expected: 43,
    },
    Program {
        name:     "float_structs_arrays_and_pointers",
        source:   "struct vec { double x, y; float w; };
static void scale(struct vec *v, double k) { v->x *= k; v->y *= k; v->w += 1; }
int main(void) {
  struct vec v = { 1.5, 2, 0.5f };
  double *p = &v.y;
  scale(&v, 2);
  *p += 0.5;
  double a[3] = { 1, 2.5, 3 };
  return (int)(v.x + v.y + v.w + a[1] * a[2]);
}",
        expected: 16,
    },
    Program {
        name:     "float_long_double_arithmetic",
        source:   "int main(void) {
  long double ld = 2;
  ld = ld * 1.5L + 0.25L;
  float f = (float)ld;
  return (int)(ld * 4) + (int)f + (sizeof(long double) >= 8) * 100;
}",
        expected: 116,
    },
    Program {
        name:     "float_ternary_and_old_style_parameters",
        source:   "static double pick(int c, double a, float b) { return c ? a : b; }
static float halve(x) float x; { return x / 2; }
int main(void) { return (int)(pick(1, 4.5, 1.5f) + pick(0, 4.5, 1.5f) * 10 + halve(5.0)); }",
        expected: 22,
    },
    Program {
        name:     "float_static_locals_and_compound_literals",
        source:   "static double next(void) { static double n = 0.5; n += 1; return n; }
int main(void) {
  double a = next();
  double b = next();
  double *p = (double[]){ 1.5, 2.5 };
  return (int)(a * 10 + b * 10 + p[0] + p[1]);
}",
        expected: 44,
    },
    Program {
        name:     "float_integer_widths_convert_exactly",
        source:   "int main(void) {
  unsigned char c = 200;
  float f = c;
  short s = -3;
  double d = s;
  unsigned u = 4000000000u;
  double du = u;
  long long big = 1ll << 40;
  float fb = big;
  return (int)f + (int)d + (du > 3.9e9) + (int)(fb / (1ll << 38)) + ((unsigned)du == u);
}",
        expected: 203,
    },
];

pub(super) const BIT_FIELD_PROGRAMS: &[Program] = &[
    Program {
        name:     "bit_field_pack_and_unpack",
        source:   "struct packed { unsigned a : 3; unsigned b : 5; unsigned c : 8; unsigned d : \
                   16; };
int main(void) {
  struct packed p;
  p.a = 5;
  p.b = 17;
  p.c = 200;
  p.d = 40000;
  return (p.a == 5) + (p.b == 17) * 2 + (p.c == 200) * 4 + (p.d == 40000) * 8 +
         (sizeof p == 4) * 16;
}",
        expected: 31,
    },
    Program {
        name:     "bit_field_sign_extension",
        source:   "struct s { int a : 4; int b : 4; signed char c : 3; };
int main(void) {
  struct s v;
  v.a = -3;
  v.b = 7;
  v.c = -4;
  int r = 0;
  if (v.a == -3) r += 1;
  if (v.b == 7) r += 2;
  if (v.c == -4) r += 4;
  v.a = 9;
  if (v.a == -7) r += 8;
  if (v.b == 7 && v.c == -4) r += 16;
  return r;
}",
        expected: 31,
    },
    Program {
        name:     "bit_field_unsigned_wraparound",
        source:   "struct u { unsigned x : 3; unsigned y : 29; };
int main(void) {
  struct u v = { 0, 0 };
  v.x = 7;
  v.x++;
  int a = v.x;
  v.x += 10;
  int b = v.x;
  v.x -= 3;
  int c = v.x;
  v.y = 0x1fffffff;
  v.y++;
  int d = v.y;
  v.x = 7;
  int e = v.x++;
  e += ++v.x;
  return a + b * 2 + c * 4 + d + e;
}",
        expected: 40,
    },
    Program {
        name:     "bit_field_initializers",
        source:   "struct pack { unsigned a : 4, b : 4; int c : 7; char d; unsigned e : 1; };
struct pack g = { 3, 15, -2, 'x', 1 };
int main(void) {
  struct pack l = { 9, 2, -1, 'y', 0 };
  struct pack n = { .e = 1, .a = 6 };
  return g.a + g.b + g.c + (g.d == 'x') + g.e + l.a + l.b + l.c + (l.d == 'y') + l.e + n.a +
         n.e + n.b;
}",
        expected: 36,
    },
    Program {
        name:     "bit_field_wide_fields",
        source:   "struct w { long long a : 40; unsigned long long b : 60; int pad; };
int main(void) {
  struct w x = { 0, 0, 0 };
  x.a = -1;
  x.b = 1;
  x.b <<= 59;
  x.a += 2;
  return (x.a == 1) + (x.b == (1ull << 59)) * 2 + (x.b >> 58 == 2) * 4 + (x.pad == 0) * 8;
}",
        expected: 15,
    },
    Program {
        name:     "bit_field_array_of_structs",
        source:   "struct r { unsigned lo : 4; unsigned hi : 4; };
int main(void) {
  struct r a[3];
  for (int i = 0; i < 3; i++) {
    a[i].lo = i + 1;
    a[i].hi = 15 - i;
  }
  int s = 0;
  for (int i = 0; i < 3; i++) s += a[i].lo * a[i].hi;
  return s;
}",
        expected: 82,
    },
    Program {
        name:     "bit_field_bool_and_unnamed_fields",
        source:   "struct f { _Bool a : 1; _Bool b : 1; int : 0; int c : 3; };
int main(void) {
  struct f x = { 0, 0, 0 };
  x.a = 5;
  x.b = 0;
  x.c = 3;
  x.c = x.c + x.a;
  return x.a + x.b * 2 + x.c * 4 + (sizeof x == 8) * 32;
}",
        expected: 17,
    },
    Program {
        name:     "bit_field_through_pointers_and_updates",
        source:   "struct c { unsigned lo : 4; int mid : 6; unsigned hi : 22; };
static int bump(struct c *p, int n) {
  p->lo += n;
  p->mid -= 3;
  p->hi++;
  --p->lo;
  p->mid <<= 1;
  return p->lo++ + --p->mid;
}
int main(void) {
  struct c v = { 1, 2, 3 };
  int r = bump(&v, 5);
  return r + v.lo + v.mid * 2 + v.hi;
}",
        expected: 6,
    },
    Program {
        name:     "bit_field_volatile_registers",
        source:   "struct reg { unsigned enable : 1; unsigned mode : 3; unsigned count : 12; };
int main(void) {
  volatile struct reg r;
  r.enable = 1;
  r.mode = 5;
  r.count = 4095;
  r.count += 2;
  return r.enable + r.mode * 2 + r.count * 16;
}",
        expected: 27,
    },
    Program {
        name:     "bit_field_union_members",
        source:   "union u { struct { unsigned lo : 8; unsigned hi : 8; } parts; unsigned short \
                   whole; };
int main(void) {
  union u x;
  x.whole = 0;
  x.parts.lo = 0x34;
  x.parts.hi = 0x12;
  return (x.whole == 0x1234) + (x.parts.hi == 0x12) * 2;
}",
        expected: 3,
    },
    Program {
        name:     "bit_field_bitwise_compound_assignment",
        source:   "struct b { unsigned f : 5; int g : 6; };
int main(void) {
  struct b x = { 3, -5 };
  x.f |= 8;
  x.f ^= 1;
  x.f >>= 1;
  x.g &= 0x1e;
  return x.f + x.g;
}",
        expected: 31,
    },
    Program {
        name:     "bit_field_loop_counter",
        source:   "struct c { unsigned n : 3; };
int main(void) {
  struct c c = { 5 };
  int s = 0;
  while (c.n) {
    s += c.n;
    c.n--;
  }
  return s;
}",
        expected: 15,
    },
    Program {
        name:     "bit_field_odd_spans",
        source:   "struct odd {
  unsigned a : 4;
  unsigned b : 17;
  unsigned c : 11;
  unsigned long long d : 33;
  unsigned e : 7;
};
int main(void) {
  struct odd o;
  o.a = 9;
  o.b = 0x1ABCD;
  o.c = 1234;
  o.d = 0x1DEADBEEFull;
  o.e = 99;
  o.b += 1;
  o.a ^= 6;
  return (o.a == 15) + (o.b == 0x1ABCE) * 2 + (o.c == 1234) * 4 +
         (o.d == 0x1DEADBEEFull) * 8 + (o.e == 99) * 16;
}",
        expected: 31,
    },
];

/// Lowers `source` for `target` and runs `main` in the interpreter,
/// returning its status.
#[track_caller]
fn run_main(name: &str, source: &str, target: Target) -> i32 {
    let ir = lower_for(source, target)
        .unwrap_or_else(|errors| panic!("{name} failed to lower for {target:?}: {errors:?}"));
    let run = run_text(&ir, "main");
    match run.result {
        | Ok(Outcome {
            termination: Termination::Returned(Some(RuntimeValue::Int(bits))),
            ..
        }) => i32::from_le_bytes(bits.to_le_bytes()[..4].try_into().unwrap()),
        | Ok(Outcome {
            termination: Termination::Exited(status),
            ..
        }) => status,
        | other => panic!(
            "{name} did not return a status on {target:?}: {other:?} {:?}\n{ir}",
            run.message
        ),
    }
}

fn targets_for(source: &str) -> &'static [Target] {
    if source.contains("long double") {
        &[Target::WindowsMsvc]
    } else {
        &TARGETS
    }
}

#[test]
fn numeric_programs_lower_to_verified_ir_on_every_target() {
    for program in FLOAT_PROGRAMS.iter().chain(BIT_FIELD_PROGRAMS) {
        for target in TARGETS {
            if let Err(errors) = lower_for(program.source, target) {
                panic!("{} failed for {target:?}: {errors:?}", program.name);
            }
        }
    }
}

#[test]
fn numeric_programs_return_their_status_in_the_interpreter() {
    for program in FLOAT_PROGRAMS.iter().chain(BIT_FIELD_PROGRAMS) {
        for &target in targets_for(program.source) {
            assert_eq!(
                run_main(program.name, program.source, target),
                program.expected,
                "{} on {target:?}",
                program.name
            );
        }
    }
}

#[test]
fn numeric_program_names_are_distinct() {
    let mut names: Vec<_> = FLOAT_PROGRAMS
        .iter()
        .chain(BIT_FIELD_PROGRAMS)
        .map(|program| program.name)
        .collect();
    names.sort_unstable();
    names.dedup();
    assert_eq!(names.len(), FLOAT_PROGRAMS.len() + BIT_FIELD_PROGRAMS.len());
}

/// The target the tests run natively on, if the back end supports it.
fn host() -> Option<Target> {
    if cfg!(all(windows, target_env = "gnu")) {
        Some(Target::WindowsGnu)
    } else if cfg!(all(windows, target_env = "msvc")) {
        Some(Target::WindowsMsvc)
    } else if cfg!(all(target_os = "linux", target_env = "gnu")) {
        Some(Target::LinuxGnu)
    } else if cfg!(all(target_os = "linux", target_env = "musl")) {
        Some(Target::LinuxMusl)
    } else {
        None
    }
}

/// Compiles the program for the host through LLVM at `-O2` and with LLVM's
/// passes disabled, runs both, and returns their exit statuses.
fn run_through_llvm(
    _toolchain: &Toolchain,
    directory: &crate::test_support::TempDir,
    program: &Program,
    target: Target,
) -> [i32; 2] {
    let ir = lower_for(program.source, target)
        .unwrap_or_else(|errors| panic!("{} failed to lower: {errors:?}", program.name));
    let arena = Bump::new();
    let mut module = parse_module(&arena, &ir).unwrap();
    // The driver's order: ABI lowering before the back end.
    crate::abi::lower_abi(&mut module, target, &arena).unwrap();
    let errors = verify_module(&module, Profile::PostAbi, &arena);
    assert!(errors.is_empty(), "{}: {errors:?}", program.name);
    [false, true].map(|disable_llvm_passes| {
        let executable = directory.join(format!("{}-{disable_llvm_passes}.exe", program.name));
        let options = LlvmOptions {
            opt: Some(OptLevel::O2),
            disable_llvm_passes,
            target,
            kind: OutputKind::Executable,
        };
        compile_module(&module, &executable, options)
            .unwrap_or_else(|error| panic!("{}: {error}", program.name));
        crate::test_support::native::run_native(&mut std::process::Command::new(&executable))
            .unwrap_or_else(|error| panic!("{}: {error}\n{ir}", program.name))
            .status
            .code()
            .unwrap()
    })
}

#[test]
fn numeric_programs_return_their_status_when_compiled_by_llvm() {
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
    let directory = crate::test_support::TempDir::new("lowering-numeric");
    std::thread::scope(|scope| {
        let runs: Vec<_> = FLOAT_PROGRAMS
            .iter()
            .chain(BIT_FIELD_PROGRAMS)
            .map(|program| {
                let (toolchain, directory) = (&toolchain, &directory);
                scope.spawn(move || {
                    (
                        program,
                        run_through_llvm(toolchain, directory, program, target),
                    )
                })
            })
            .collect();
        for run in runs {
            let (program, statuses) = run.join().unwrap();
            assert_eq!(
                statuses, [program.expected; 2],
                "{} through clang on {target:?}",
                program.name
            );
        }
    });
}

#[test]
fn numeric_programs_keep_their_status_after_optimization() {
    use crate::optimizer::{
        OptimizerOptions,
        optimize,
    };
    for program in FLOAT_PROGRAMS.iter().chain(BIT_FIELD_PROGRAMS) {
        for &target in targets_for(program.source) {
            let ir = lower_for(program.source, target).unwrap();
            let arena = Bump::new();
            let scratch = Bump::new();
            let mut module = parse_module(&arena, &ir).unwrap();
            let mut options = OptimizerOptions::default();
            options.verify_each = true;
            _ = optimize(&mut module, &options, &scratch);
            let optimized = module.to_string();
            let run = run_text(&optimized, "main");
            let status = match run.result {
                | Ok(Outcome {
                    termination: Termination::Returned(Some(RuntimeValue::Int(bits))),
                    ..
                }) => i32::from_le_bytes(bits.to_le_bytes()[..4].try_into().unwrap()),
                | other => panic!(
                    "{} did not return after optimization: {other:?} {:?}\n{optimized}",
                    program.name, run.message
                ),
            };
            assert_eq!(
                status, program.expected,
                "{} on {target:?} after optimization",
                program.name
            );
        }
    }
}
