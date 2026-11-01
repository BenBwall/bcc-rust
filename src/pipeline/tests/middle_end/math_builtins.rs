//! C99 math aliases and header constants through the interpreter, LLVM and
//! native x86-64 back ends. Native libm is an independent numeric reference.

use super::*;

fn math_program(long_double: bool) -> String {
    let fixture = include_str!("../../../../tests/fixtures/programs/host_math.c");
    let (_, body) = fixture.split_once("static int test_float(void)").unwrap();
    let mut source = format!("int printf(const char *, ...);\nstatic int test_float(void){body}");
    let mut aliases = String::new();
    for token in source.split_inclusive(|c: char| !c.is_ascii_alphanumeric() && c != '_') {
        if let Some(name) = token.strip_suffix('(') {
            if let Some(descriptor) = crate::builtins::ordinary(name) {
                if name != "printf" {
                    aliases.push_str(descriptor.spelling);
                    aliases.push('(');
                    continue;
                }
            } else if matches!(name, "copysign" | "copysignf" | "copysignl") {
                aliases.push_str("__builtin_");
                aliases.push_str(token);
                continue;
            }
        }
        aliases.push_str(token);
    }
    source = aliases;
    if !long_double {
        source.insert_str(0, "#define NO_LONG_DOUBLE\n");
    }
    source
}

const CONSTANTS_AND_ROTATES: &str = r#"
static float inf_f = __builtin_inff(), huge_f = __builtin_huge_valf(), nan_f = __builtin_nanf("0x123");
static double inf_d = __builtin_inf(), huge_d = __builtin_huge_val(), nan_d = __builtin_nan("291");
static long double inf_l = __builtin_infl(), huge_l = __builtin_huge_vall(), nan_l = __builtin_nanl("0443");
static int calls;
char *tag(void) { ++calls; return "0x123"; }
float square(float x) { return __builtin_sqrtf(x); }
int main(void) {
    if (!__builtin_isinf(inf_f) || !__builtin_isinf(huge_f) || !__builtin_isnanf(nan_f)) return 1;
    if (!__builtin_isinf(inf_d) || !__builtin_isinf(huge_d) || !__builtin_isnan(nan_d)) return 2;
#ifndef NO_LONG_DOUBLE
    if (!__builtin_isinf(inf_l) || !__builtin_isinf(huge_l) || !__builtin_isnanl(nan_l)) return 3;
#endif
    union { float f; unsigned u; } f = { nan_f };
    union { double d; unsigned long long u; } d = { nan_d };
    if (f.u != 0x7fc00123u || d.u != 0x7ff8000000000123ull) return 4;
    double dynamic = __builtin_nan(tag());
    if (!__builtin_isnan(dynamic) || calls != 1) return 5;
    dynamic = (__builtin_choose_expr(1, __builtin_nan, __builtin_nan))(tag());
    if (!__builtin_isnan(dynamic) || calls != 2) return 14;
    volatile unsigned x = 0x12345678u, n = 0;
    if (__builtin_rotateleft32(x, n) != x || __builtin_rotateright32(x, n) != x) return 6;
    n = 32;
    if (__builtin_rotateleft32(x, n) != x || __builtin_rotateright32(x, n) != x) return 7;
    n = 36;
    if (__builtin_rotateleft32(x, n) != 0x23456781u || __builtin_rotateright32(x, n) != 0x81234567u) return 8;
    if (__builtin_rotateleft8(0x81, 1) != 3 || __builtin_rotateright8(3, 1) != 0x81) return 9;
    if (__builtin_rotateleft16(0x8001, 17) != 3 || __builtin_rotateright16(3, 17) != 0x8001) return 10;
    if (__builtin_rotateleft64(0x8000000000000001ull, 65) != 3 || __builtin_rotateright64(3, 65) != 0x8000000000000001ull) return 11;
    n = (unsigned)-4;
    if (__builtin_rotateleft32(x, n) != 0x81234567u) return 12;
    volatile float a = 0x1.000002p0f;
    if (__builtin_fmaf(a, a, -0x1.000004p0f) != 0x1p-46f) return 13;
    { int (*__builtin_sqrtf)(int) = 0; }
    return square(4.0f) != 2.0f;
}
"#;

#[test]
fn math_builtins_execute_in_all_interpreter_stages() {
    for target in [
        Target::LinuxGnu,
        Target::LinuxMusl,
        Target::WindowsGnu,
        Target::WindowsMsvc,
    ] {
        let long_double = target == Target::WindowsMsvc;
        let source = math_program(long_double);
        let constants = format!(
            "{}{}",
            if long_double {
                ""
            } else {
                "#define NO_LONG_DOUBLE\n"
            },
            CONSTANTS_AND_ROTATES
        );
        for (optimize, profile) in STAGES {
            let output = with_module(&source, target, optimize, profile, interpret);
            assert_eq!(
                output.0, 0,
                "{target:?} {optimize} {profile:?}: {}",
                output.1
            );
            assert!(
                output.1.lines().all(|line| line.ends_with(" 1")),
                "{}",
                output.1
            );
            let output = with_module(&constants, target, optimize, profile, interpret);
            assert_eq!(output.0, 0, "constants {target:?} {optimize} {profile:?}");
        }
    }
}

#[test]
fn math_builtins_execute_in_llvm_and_native_arms() {
    let Some(target) = host() else { return };
    let toolchain = Toolchain::bundled(target).unwrap();
    let directory = TempDir::new("math-builtins");
    for native in [false, true] {
        // x87 execution is supported by LLVM; native's declared scalar
        // contract and interpreter support binary32/64 only.
        let source = math_program(!native);
        let constants = format!(
            "{}{}",
            if native {
                "#define NO_LONG_DOUBLE\n"
            } else {
                ""
            },
            CONSTANTS_AND_ROTATES
        );
        for (name, source) in [("families", &source), ("constants", &constants)] {
            for arm in ARMS {
                let object = directory.join("program.o");
                let executable = directory.join(format!("{name}-{native}-{}.exe", arm.name));
                let options = LlvmOptions {
                    target,
                    opt: Some(arm.opt),
                    disable_llvm_passes: arm.disable_llvm_passes,
                    kind: OutputKind::Object,
                    code: crate::target::CodeOptions::default(),
                    fast_math_runtime: false,
                };
                with_module(source, target, arm.bcc, Profile::PostAbi, |module, _| {
                    if native {
                        let mut assembly = String::new();
                        crate::backend::native::emit_assembly(module, target, &mut assembly)
                            .unwrap();
                        crate::backend::native::assemble(
                            &assembly,
                            &object,
                            target,
                            OutputKind::Object,
                        )
                        .unwrap();
                    } else {
                        compile_module(module, &object, options).unwrap();
                    }
                });
                let mut link = toolchain.command(
                    &object,
                    &executable,
                    LlvmOptions {
                        kind: OutputKind::Executable,
                        ..options
                    },
                );
                let result = crate::test_support::native::run_native(&mut link).unwrap();
                assert!(
                    result.status.success(),
                    "{}",
                    String::from_utf8_lossy(&result.stderr)
                );
                let output = run_native(&executable);
                assert_eq!(
                    output.0, 0,
                    "{name} native={native} {}: {}",
                    arm.name, output.1
                );
                assert!(
                    output.1.lines().all(|line| line.ends_with(" 1")),
                    "{}",
                    output.1
                );
            }
        }
    }
}
