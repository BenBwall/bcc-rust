//! GNU `transparent_union` arguments (#68): System V passes one as its first
//! member, so a union whose first member is a structure of floating members
//! travels in vector registers although the whole union classifies as
//! INTEGER; the Microsoft x64 convention passes the whole union. Results
//! are always the whole union.
use super::*;

/// The types both sides share. `F` and `D` are the examples GCC and Clang
/// pass in `%xmm0`; `A` is a pointer union an `aligned` attribute makes
/// larger than its first member, which System V still passes as a pointer,
/// and `UA` such a union of `F`'s members, which goes on the stack whole
/// (32 bytes aligned to 32) once the vector registers are taken. `UC` and
/// `UB` have narrow scalar first members, which System V extends as
/// scalar arguments (Clang: `i8 signext`, `i1 zeroext`).
const TYPES: &str = "\
struct FP { float x, y; };
struct IP { int a, b; };
struct DS { double d; };
typedef union { struct FP s; struct IP t; } __attribute__((transparent_union)) F;
typedef union { struct DS s; long long l; } __attribute__((transparent_union)) D;
typedef union { int *p; long *l; } __attribute__((transparent_union, aligned(16))) A;
typedef union { struct FP s; struct IP t; } __attribute__((transparent_union, aligned(32))) UA;
typedef union { signed char c; unsigned char u; } __attribute__((transparent_union)) UC;
typedef union { _Bool b; char c; } __attribute__((transparent_union)) UB;
";

/// Callees on one side, written once with `X_` for the side's prefix.
const CALLEES: &str = "\
float X_f(F u) { return u.s.x * 10 + u.s.y; }
double X_d(D u) { return u.s.d * 2; }
int X_a(A u) { return *u.p + 1; }
double X_many(double a0, double a1, double a2, double a3, double a4, double a5, double a6,
              double a7, F u, int i0, int i1, int i2, int i3, int i4, int i5, A a, D v) {
  return a0 + a7 + u.s.x + u.s.y + v.s.d + *a.p + i0 + i5;
}
F X_ret(float x, float y) { F u; u.s.x = x; u.s.y = y; return u; }
float X_ua(UA u) { return u.s.x * 10 + u.s.y; }
float X_ua_stack(double a0, double a1, double a2, double a3, double a4, double a5, double a6, \
                       double a7, UA u, int i, UA v) { return a0 + a7 + u.s.x * 10 + u.s.y + i + \
                       v.s.x * 100 + v.s.y; }
int X_uc(UC u) { return u.c; }
int X_ub(UB u) { return u.b ? 7 : 3; }
int X_narrow_stack(long a0, long a1, long a2, long a3, long a4, long a5, UC u, UB b) {
  return (int)(a0 + a5) + u.c * 10 + (b.b ? 1000 : 0);
}
";

/// The calls one side makes of the other's callees (prefix `Y_`), which
/// return 0 or the number of the first failed check; named for the callees.
const CALLS: &str = "\
int Y_calls(void) {
  struct FP fp = { 1.5f, 2.25f };
  struct DS ds = { 4.5 };
  int seven = 7;
  F f = { fp };
  D d = { ds };
  if (Y_f(fp) != 17.25f || Y_f(f) != 17.25f) return 1;
  if (Y_d(ds) != 9.0 || Y_d(d) != 9.0) return 2;
  if (Y_a(&seven) != 8) return 3;
  if (Y_many(1, 2, 3, 4, 5, 6, 7, 8, fp, 10, 11, 12, 13, 14, 15, &seven, ds) != 49.25) return 4;
  F r = Y_ret(0.5f, 0.25f);
  if (r.s.x != 0.5f || r.s.y != 0.25f) return 5;
  UA ua = { { 0.5f, 0.25f } };
  if (Y_ua(fp) != 17.25f || Y_ua(ua) != 5.25f) return 6;
  if (Y_ua_stack(1, 2, 3, 4, 5, 6, 7, 8, ua, 20, fp) != 186.5f) return 7;
  signed char minus = -3;
  unsigned char byte = 0xfd;
  if (Y_uc(minus) != -3 || Y_uc(byte) != -3) return 8;
  _Bool yes = 1;
  if (Y_ub(yes) != 7 || Y_ub((_Bool)0) != 3) return 9;
  if (Y_narrow_stack(1, 2, 3, 4, 5, 6, minus, yes) != 977) return 10;
  return 0;
}
";

/// Declarations of the other side's callees, with `Y_` for its prefix.
const DECLARATIONS: &str = "\
float Y_f(F u);
double Y_d(D u);
int Y_a(A u);
double Y_many(double a0, double a1, double a2, double a3, double a4, double a5, double a6,
              double a7, F u, int i0, int i1, int i2, int i3, int i4, int i5, A a, D v);
F Y_ret(float x, float y);
float Y_ua(UA u);
float Y_ua_stack(double a0, double a1, double a2, double a3, double a4, double a5, double a6, \
                            double a7, UA u, int i, UA v);
int Y_uc(UC u);
int Y_ub(UB u);
int Y_narrow_stack(long a0, long a1, long a2, long a3, long a4, long a5, UC u, UB b);
";

/// A variadic callee and its caller: `...` passes a transparent union as
/// its first member too, and `va_arg` reads it so; once the vector
/// registers are taken, `UA` comes from the overflow area at a 32-byte
/// boundary and the cursor moves past all 32 bytes.
const VARIADIC: &str = "\
double X_vsum(int n, ...) {
  __builtin_va_list ap;
  __builtin_va_start(ap, n);
  double sum = 0;
  for (int i = 0; i < n; i++) {
    F f = __builtin_va_arg(ap, F);
    D d = __builtin_va_arg(ap, D);
    sum += f.s.x + f.s.y + d.s.d;
  }
  __builtin_va_end(ap);
  return sum;
}
double X_vua(int n, ...) {
  __builtin_va_list ap;
  __builtin_va_start(ap, n);
  double sum = 0;
  for (int i = 0; i < n; i++)
    sum += __builtin_va_arg(ap, double);
  UA u = __builtin_va_arg(ap, UA);
  int k = __builtin_va_arg(ap, int);
  UA v = __builtin_va_arg(ap, UA);
  __builtin_va_end(ap);
  return sum + u.s.x * 10 + u.s.y + k + v.s.x * 100 + v.s.y;
}
";

const VARIADIC_CALL: &str = "\
double Y_vsum(int n, ...);
double Y_vua(int n, ...);
int Y_vcalls(void) {
  F f = { { 1.5f, 2.25f } };
  D d = { { 4.5 } };
  UA u = { { 0.5f, 0.25f } };
  UA v = { { 3.0f, 0.75f } };
  if (Y_vsum(2, f, d, f, d) != 16.5) return 1;
  if (Y_vua(8, 1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, u, 20, v) != 362.0) return 2;
  return 0;
}
";

fn side(text: &str, own: &str, other: &str) -> String {
    text.replace("X_", own).replace("Y_", other)
}

/// The bcc and Clang sources, with the variadic part when `variadic`. The
/// Clang side's `main` runs the calls in both directions.
fn sources(variadic: bool) -> (String, String) {
    sources_of(variadic, true)
}

/// [`sources`], without the over-aligned union `UA` unless `aligned`: the
/// native back end refuses stack objects aligned above 16 bytes.
fn sources_of(variadic: bool, aligned: bool) -> (String, String) {
    let keep = |text: &str| -> String {
        text.lines()
            .filter(|line| aligned || !(line.contains("UA") || line.contains("_ua")))
            .flat_map(|line| [line, "\n"])
            .collect()
    };
    let mut bcc = format!(
        "{TYPES}{}{}{}",
        keep(&side(CALLEES, "b_", "c_")),
        keep(&side(DECLARATIONS, "b_", "c_")),
        keep(&side(CALLS, "b_", "c_")),
    );
    let mut clang = format!(
        "{TYPES}{}{}{}int c_calls(void);\n",
        keep(&side(CALLEES, "c_", "b_")),
        keep(&side(DECLARATIONS, "c_", "b_")),
        keep(&side(CALLS, "c_", "b_")),
    );
    let mut main = String::from(
        "int main(void) {\n  int status = c_calls();\n  if (status) return 10 + status;\n  if \
         ((status = b_calls())) return 20 + status;\n",
    );
    if variadic {
        bcc.push_str(&side(VARIADIC, "b_", "c_"));
        bcc.push_str(&side(VARIADIC_CALL, "b_", "c_"));
        clang.push_str(&side(VARIADIC, "c_", "b_"));
        clang.push_str(&side(VARIADIC_CALL, "c_", "b_"));
        clang.push_str("int c_vcalls(void);\n");
        main.push_str("  if (c_vcalls()) return 31;\n  if (b_vcalls()) return 32;\n");
    }
    main.push_str("  return 0;\n}\n");
    clang.push_str(&main);
    (bcc, clang)
}

/// The post-ABI signature line of `@name` in `module`.
fn function_line(module: &Module<'_>, name: &str) -> String {
    let text = module.to_string();
    text.lines()
        .find(|line| line.starts_with(&format!("function @{name}(")))
        .unwrap_or_else(|| panic!("no @{name}\n{text}"))
        .to_string()
}

/// The post-ABI parameter list of `@name` in `module`.
fn parameters(module: &Module<'_>, name: &str) -> String {
    let line = function_line(module, name);
    line[line.find('(').unwrap()..=line.find(')').unwrap()].to_string()
}

#[test]
fn transparent_union_arguments_take_their_first_members_registers_on_system_v() {
    let (bcc, _) = sources(true);
    for target in [Target::LinuxGnu, Target::WindowsGnu, Target::WindowsMsvc] {
        let tu = Bump::new();
        let arena = Bump::new();
        let mut module = lower_c(&tu, &arena, &bcc, target);
        lower_abi(&mut module, target, &Bump::new()).unwrap();
        assert_verified(&module, Profile::PostAbi, &module.to_string());
        let (f, d, a) = (
            parameters(&module, "b_f"),
            parameters(&module, "b_d"),
            parameters(&module, "b_a"),
        );
        if target == Target::LinuxGnu {
            // Clang: `float @ff(<2 x float>)`, `double @fd(double)`,
            // `i32 @fa(ptr)`.
            assert_eq!(f, "(f64)", "{module}");
            assert_eq!(d, "(f64)", "{module}");
            assert_eq!(a, "(ptr)", "{module}");
            // Clang: `@ua(<2 x float>)`, and after eight doubles
            // `ptr byval(%union.UA) align 32` for the whole union;
            // `i8 signext` and `i1 zeroext` for the narrow first members.
            assert_eq!(parameters(&module, "b_ua"), "(f64)", "{module}");
            let stack = function_line(&module, "b_ua_stack");
            assert!(
                stack.contains("byval(size 32, align 32)"),
                "{stack}\n{module}"
            );
            assert_eq!(parameters(&module, "b_uc"), "(i8 signext)", "{module}");
            assert_eq!(parameters(&module, "b_ub"), "(i8 zeroext)", "{module}");
        } else {
            // Clang: `float @ff(i64)`, `double @fd(i64)`, and the 16-byte
            // union by reference; narrow unions are plain `i8`.
            assert_eq!(f, "(i64)", "{module}");
            assert_eq!(d, "(i64)", "{module}");
            assert_eq!(a, "(ptr noundef)", "{module}");
            assert_eq!(parameters(&module, "b_uc"), "(i8)", "{module}");
            assert_eq!(parameters(&module, "b_ub"), "(i8)", "{module}");
        }
    }
}

#[test]
fn transparent_union_arguments_cross_between_bcc_and_clang() {
    let (bcc, clang) = sources(true);
    for target in [Target::LinuxGnu, Target::WindowsGnu, Target::WindowsMsvc] {
        for optimized in [false, true] {
            let tu = Bump::new();
            let arena = Bump::new();
            let mut module = lower_c(&tu, &arena, &bcc, target);
            if optimized {
                _ = optimize(&mut module, &OptimizerOptions::default(), &Bump::new());
            }
            lower_abi(&mut module, target, &Bump::new()).unwrap();
            assert_verified(&module, Profile::PostAbi, &module.to_string());
            assert_conforms("transparent-unions", target, &module, &clang);
        }
    }
}

#[test]
fn transparent_union_arguments_cross_between_native_code_and_clang() {
    let (bcc, clang) = sources_of(false, false);
    for target in [Target::LinuxGnu, Target::WindowsGnu] {
        for optimized in [false, true] {
            let tu = Bump::new();
            let arena = Bump::new();
            let mut module = lower_c(&tu, &arena, &bcc, target);
            if optimized {
                _ = optimize(&mut module, &OptimizerOptions::default(), &Bump::new());
            }
            lower_abi(&mut module, target, &Bump::new()).unwrap();
            assert_native_conforms("transparent-unions-native", target, &module, &clang);
        }
    }
}

#[test]
fn transparent_union_arguments_run_in_the_interpreter() {
    use crate::backend::interpreter::{
        DefaultHost,
        RuntimeValue,
        Termination,
        run,
    };
    // Both sides in one module: the callee reads what the caller wrote
    // through the first-member convention, before and after ABI lowering.
    // Each side's prototypes of the other's functions are redeclarations.
    let (bcc, clang) = sources(true);
    let source = format!("{bcc}{}", clang.strip_prefix(TYPES).unwrap());
    for target in [Target::LinuxGnu, Target::WindowsGnu, Target::WindowsMsvc] {
        for profile in [Profile::PreAbi, Profile::PostAbi] {
            let tu = Bump::new();
            let arena = Bump::new();
            let mut module = lower_c(&tu, &arena, &source, target);
            if profile == Profile::PostAbi {
                lower_abi(&mut module, target, &Bump::new()).unwrap();
            }
            let execution = Bump::new();
            let mut host = DefaultHost::for_target(&execution, target);
            let result = run(&module, "main", &[], &mut host)
                .unwrap_or_else(|error| panic!("{target:?} {profile:?}: {error:?}\n{module}"));
            assert_eq!(
                result.termination,
                Termination::Returned(Some(RuntimeValue::Int(0))),
                "{target:?} {profile:?}"
            );
        }
    }
}
