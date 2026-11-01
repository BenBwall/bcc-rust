//! Aggregate varargs preserve declared alignment at every interpreter stage.
use super::*;

const ALIGNMENT_REGRESSIONS: &[&str] = &[
    "typedef struct { int x; } S __attribute__((aligned(64)));
int take(int n,...) { __builtin_va_list ap; __builtin_va_start(ap,n);
S b=__builtin_va_arg(ap,S); __builtin_va_end(ap); return b.x!=42 || (unsigned long long)&b%64; }
int main(void) { S a={42}; return take(0,a); }",
    "typedef struct { int x; } S __attribute__((aligned(64)));
int take(S a) { S b=a; return b.x!=42 || (unsigned long long)&a%64 || (unsigned long long)&b%64; }
int main(void) { S a={42}; return take(a); }",
    "int f(int n){typedef int Row[n] __attribute__((aligned(64))); Row rows[2];
rows[1][0]=42; return rows[1][0]!=42;} int main(void){return f(3);}",
];

const EMPTY_CASE_RANGE: &str = r"
int f(int n) { int r=0; switch(n) {
case 1: r+=1;
case 9 ... 3: r+=2;
case 2: r+=4; break;
case 20 ... 10: r+=8;
default: r+=16; }
return r; }
int main(void) { return f(1)!=7 || f(2)!=4 || f(3)!=16 || f(9)!=16 || f(10)!=16 || f(20)!=16; }
";

#[test]
fn typedef_alignment_va_arg_backing_storage() {
    check_storage_alignment(ALIGNMENT_REGRESSIONS[0]);
}

#[test]
fn typedef_alignment_parameter_homes() {
    check_storage_alignment(ALIGNMENT_REGRESSIONS[1]);
}

#[test]
fn variable_array_stride_refines_storage_alignment() {
    check_storage_alignment(ALIGNMENT_REGRESSIONS[2]);
}

fn check_storage_alignment(source: &str) {
    for target in [Target::LinuxGnu, Target::WindowsGnu, Target::WindowsMsvc] {
        for (optimize, profile) in STAGES {
            with_module(source, target, optimize, profile, |module, target| {
                assert_eq!(
                    interpret(module, target).0,
                    0,
                    "{target:?} {optimize} {profile:?}"
                );
            });
        }
    }
}

#[test]
fn overflow_area_retains_the_strongest_argument_alignment() {
    let source = "struct __attribute__((aligned(32))) A { long long x[4]; };
struct __attribute__((aligned(64))) B { long long x[4]; };
int take(int n, ...) { __builtin_va_list ap; __builtin_va_start(ap,n);
struct A a=__builtin_va_arg(ap,struct A); struct B b=__builtin_va_arg(ap,struct B);
int s=__builtin_va_arg(ap,int); __builtin_va_end(ap);
return a.x[0]!=7 || a.x[3]!=11 || b.x[0]!=13 || b.x[3]!=17 || s!=123; }
int main(void) { struct A a={7,8,9,11}; struct B b={13,14,15,17};
return take(0,a,b,123); }";
    for target in [Target::LinuxGnu, Target::WindowsGnu, Target::WindowsMsvc] {
        for (optimize, profile) in STAGES {
            with_module(source, target, optimize, profile, |module, target| {
                assert_eq!(interpret(module, target).0, 0);
            });
        }
    }
}

#[test]
fn over_aligned_aggregate_varargs_execute_in_all_stages() {
    for alignment in [32, 64] {
        let source = format!(
            "struct __attribute__((aligned({alignment}))) S {{ long long x[4]; }};
int take(int n, ...) {{ __builtin_va_list ap, cp; __builtin_va_start(ap,n);
__builtin_va_copy(cp,ap); struct S a=__builtin_va_arg(ap,struct S);
struct S b=__builtin_va_arg(cp,struct S); int s=__builtin_va_arg(ap,int);
__builtin_va_end(ap); __builtin_va_end(cp);
return a.x[0]!=7 || a.x[3]!=11 || b.x[0]!=7 || b.x[3]!=11 || s!=123; }}
int main(void) {{ struct S x={{7,8,9,11}}; return take(0,x,123); }}"
        );
        for target in [Target::LinuxGnu, Target::WindowsGnu, Target::WindowsMsvc] {
            for (optimize, profile) in STAGES {
                with_module(&source, target, optimize, profile, |module, target| {
                    assert_eq!(
                        interpret(module, target).0,
                        0,
                        "{target:?} {optimize} {profile:?}"
                    );
                });
            }
        }
    }
}

#[test]
fn aggregate_varargs_use_nominal_alignment() {
    let source = "struct S { long a,b,c; };
int take(int n,...) { __builtin_va_list ap; __builtin_va_start(ap,n);
for(int i=0;i<6;i++) if(__builtin_va_arg(ap,int)!=i+1)return 2;
typedef struct S T __attribute__((aligned(32)));
struct S x=__builtin_va_arg(ap,T); int s=__builtin_va_arg(ap,int);
__builtin_va_end(ap); return x.a!=7 || x.b!=8 || x.c!=9 || s!=123; }
int main(void) { struct S x __attribute__((aligned(64)))={7,8,9};
return take(0,1,2,3,4,5,6,x,123); }";
    for target in [Target::LinuxGnu, Target::WindowsGnu, Target::WindowsMsvc] {
        for (optimize, profile) in STAGES {
            with_module(source, target, optimize, profile, |module, target| {
                assert_eq!(
                    interpret(module, target).0,
                    0,
                    "{target:?} {optimize} {profile:?}"
                );
            });
        }
    }
}

#[test]
fn round4_empty_case_range_fallthrough() {
    for target in [Target::LinuxGnu, Target::WindowsGnu, Target::WindowsMsvc] {
        for (optimize, profile) in STAGES {
            with_configured_module(
                EMPTY_CASE_RANGE,
                target,
                optimize,
                profile,
                CompilerConfiguration::default()
                    .with_target(target)
                    .with_gnu_extensions(true),
                |module, target| {
                    assert_eq!(interpret(module, target).0, 0);
                },
            );
        }
    }
}

#[test]
fn round4_storage_alignment_matches_bundled_clang() {
    let directory = TempDir::new("round4-alignment");
    // Cross-target IR needs only Clang; native linking uses the host sysroot.
    let toolchain = Toolchain::bundled(host().unwrap_or(Target::LinuxGnu)).unwrap();
    for (index, source) in ALIGNMENT_REGRESSIONS
        .iter()
        .copied()
        .chain([EMPTY_CASE_RANGE])
        .enumerate()
    {
        let input = directory.join(format!("alignment-{index}.c"));
        std::fs::write(&input, source).unwrap();
        for target in [Target::LinuxGnu, Target::WindowsGnu, Target::WindowsMsvc] {
            for opt in [OptLevel::O0, OptLevel::O2] {
                let llvm = directory.join(format!("reference-{index}-{target:?}-{opt:?}.ll"));
                let output = crate::test_support::native::run_native(
                    Command::new(toolchain.clang())
                        .args([
                            "-std=gnu11",
                            "-S",
                            "-emit-llvm",
                            if opt == OptLevel::O0 { "-O0" } else { "-O2" },
                        ])
                        .arg(format!("--target={}", target.triple()))
                        .arg(&input)
                        .arg("-o")
                        .arg(&llvm),
                )
                .unwrap();
                assert!(
                    output.status.success(),
                    "{}",
                    String::from_utf8_lossy(&output.stderr)
                );
                if opt == OptLevel::O0 && index < ALIGNMENT_REGRESSIONS.len() {
                    let ir = std::fs::read_to_string(&llvm).unwrap();
                    if index < 2 {
                        assert!(ir.contains("alloca %struct"));
                    }
                    assert!(ir.contains("align 64"));
                }
                if host() == Some(target) {
                    let executable = directory.join(format!("reference-{index}-{opt:?}.exe"));
                    let options = LlvmOptions {
                        target,
                        opt: Some(opt),
                        disable_llvm_passes: false,
                        kind: OutputKind::Executable,
                        code: crate::target::CodeOptions::default(),
                        fast_math_runtime: false,
                    };
                    let output = crate::test_support::native::run_native(&mut toolchain.command(
                        &input,
                        &executable,
                        options,
                    ))
                    .unwrap();
                    assert!(
                        output.status.success(),
                        "{}",
                        String::from_utf8_lossy(&output.stderr)
                    );
                    assert_eq!(run_native(&executable).0, 0);
                    for optimize in [false, true] {
                        with_configured_module(
                            source,
                            target,
                            optimize,
                            Profile::PostAbi,
                            CompilerConfiguration::default()
                                .with_target(target)
                                .with_gnu_extensions(true),
                            |module, _| {
                                let executable =
                                    directory.join(format!("bcc-{index}-{opt:?}-{optimize}.exe"));
                                compile_module(module, &executable, options).unwrap();
                                assert_eq!(run_native(&executable).0, 0);
                            },
                        );
                    }
                }
            }
        }
    }
}
