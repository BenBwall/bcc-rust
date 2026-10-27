//! C99 array extents, declaration lifetime, and Clang execution comparisons.
use super::*;

const M2_PROGRAMS: &[&str] = &[
    "int count; int extent(void){return ++count+1;} int f(int n,int a[extent()][n]){
     return count!=1 || sizeof *a!=12;} int main(void){int a[2][3];return f(3,a);}",
    "int f(register int n,int (*p)[n],int a[static 3]){{int n=9;(void)n;}
     return sizeof *p!=12 || a[0]!=7;} int main(void){int a[3][3]={{7}};return f(3,a,a[0]);}",
    "int old(c,r) char c; float r; {return c!=7 || r!=2.5f;}
     int main(void){return old(7,2.5f);}",
    "int f(int,int n){return n!=7;} int main(void){return f(99,7);}",
    "int main(void){int n=2;typedef int A[n++];A a,b;n=9;
     int c[sizeof a/sizeof(int)];return n!=9 || sizeof a!=8 || sizeof b!=8 || sizeof c!=8;}",
    "int main(void){int n=2,m=3;int a[n++][m++];a[1][2]=17;
     return n!=3 || m!=4 || sizeof a!=24 || a[1][2]!=17;}",
    "int main(void){int n=2;int a[3][3];int (*p)[n++]=(int (*)[3])a;
     int before=n;unsigned long s=sizeof(int[n++]);
     return before!=3 || n!=4 || s!=12 || sizeof(n++)!=4 || n!=4 || sizeof *p!=8;}",
    "int main(void){int n=2,m=3,k=4;int a[n][m][k];int (*p)[m][k]=a;
     p++;p[0][2][3]=42;return p-a!=1 || sizeof *p!=48 || a[1][2][3]!=42 ||
     &a[1][2][3]-&a[1][0][0]!=11;}",
    "int main(void){int n=2,m=3,a[n][m];int (*p)[m]=a;unsigned long size=sizeof *p++;
     return p!=a+1 || size!=12;}",
];

#[test]
fn m2_parameter_bound_size_and_stride_execution() {
    for source in M2_PROGRAMS {
        for target in [Target::LinuxGnu, Target::WindowsGnu] {
            for (optimize, profile) in STAGES {
                with_module(source, target, optimize, profile, |module, target| {
                    assert_eq!(interpret(module, target).0, 0, "{source}");
                });
            }
        }
    }
}

const ALIGNED_VLAS: &str = "struct __attribute__((aligned(64))) S { int x; };
int main(void) { int n=3; _Alignas(32) int a[n], b[n]; struct S c[n];
a[0]=7; b[0]=11; c[0].x=13;
return (unsigned long long)a%32 || (unsigned long long)b%32 ||
(unsigned long long)c%64 || a[0]!=7 || b[0]!=11 || c[0].x!=13; }";

#[test]
fn dynamic_allocations_preserve_declared_alignment() {
    for target in [Target::LinuxGnu, Target::WindowsGnu] {
        for (optimize, profile) in STAGES {
            with_module(ALIGNED_VLAS, target, optimize, profile, |module, target| {
                assert_eq!(interpret(module, target).0, 0);
                let printed = module.to_string();
                let arena = Bump::new();
                let parsed = parse_module(&arena, &printed).unwrap();
                assert_eq!(interpret(&parsed, target).0, 0);
            });
        }
    }
}

const STATEMENT_EXPRESSION_JUMPS: &[&str] = &[
    "int main(void) { int n=4, a[n]; a[0]=42; ({ goto l; l:; 0; }); return a[0]!=42; }",
    "int main(void) { int n=4; return ({ int a[n]; a[0]=42; goto l; l:; a[0]!=42; }); }",
    "int main(void) { return ({ goto l; l:; 0; }); }",
    "int main(void) { int n=4,a[n]; a[0]=42; int r=({ int b[n]; b[0]=7; ({ goto l; l:; 0; }); \
     b[0]!=7; }); return r || a[0]!=42; }",
    "int main(void) { int n=4,a[n]; a[0]=42; ({ int b[n]; b[0]=7; ({ int c[n]; c[0]=9; goto l; \
     l:; c[0]; }); if(b[0]!=7)return 1; 0; }); return a[0]!=42; }",
    "int main(void) { int n=4,a[n]; a[0]=42; ({ int b[n]; b[0]=7; goto done; 0; }); done: return \
     a[0]!=42; }",
    "int main(void) { int n=4,a[n]; a[0]=42; int r=({ int i=0; again:; int b[n]; b[0]=7; \
     if(++i<10)goto again; b[0]!=7; }); return r || a[0]!=42; }",
];

#[test]
fn statement_expression_jumps_preserve_enclosing_vlas() {
    for source in STATEMENT_EXPRESSION_JUMPS {
        for target in [Target::LinuxGnu, Target::WindowsGnu] {
            for (optimize, profile) in STAGES {
                with_module(source, target, optimize, profile, |module, target| {
                    assert_eq!(interpret(module, target).0, 0, "{source}");
                });
            }
        }
    }
}

const BACKWARD_GOTO: &str = "int main(void) { int n=8192, i=0;
again: ; int a[n]; a[0]=i; a[n-1]=i+1; if(a[n-1]-a[0]!=1)return 1;
 if(++i<1000)goto again; return 0; }";

const VOLATILE_ESCAPE: &str = "void *volatile p; int main(void) { int n=0;
lab: ; int x[n%1000+1]; x[0]=1; x[n%1000]=2; p=x;
if(++n<1000)goto lab; return 0; }";

const INNER_LABEL: &str = "void *volatile p; int main(void) { int n=0;
if(0) { lab: ; } int x[n%1000+1]; x[0]=1; x[n%1000]=2; p=x;
if(++n<1000)goto lab; return 0; }";

const BETWEEN_ARRAYS: &str = "int main(void) { int n=8192, i=0; int a[n]; a[0]=42;
again: ; int b[n]; b[0]=i; b[n-1]=i+1; if(b[n-1]-b[0]!=1)return 1;
if(++i<1000)goto again; return a[0]!=42; }";

// GCC Statement Exprs: header break/continue targets the enclosing body.
// The outer for-init VLA survives continue, while inner/header VLAs do not.
const NESTED_HEADER_CONTINUE: &str = "int main(void){int n=128,i=0,total=0;
for(int a[n];i<1000;i++){a[0]=i;
for(int b[n];({int c[n];b[0]=a[0];c[0]=b[0];total+=c[0];continue;0;});)return 1;
return 2;}return i!=1000 || total!=499500;}";

// Reenter the two nested loops 1,000 times so a missed outer break cleanup
// cannot hide as storage released only when main returns.
const NESTED_HEADER_BREAK: &str = "int main(void){int n=128,total=0;
for(int i=0;i<1000;i++){for(int a[n];;){a[0]=i;
for(int b[n];1;({int c[n];c[0]=b[0];total+=c[0];break;0;})){b[0]=a[0];}
return 1;}}return total!=499500;}";

#[test]
fn vla_stack_storage_is_bounded_and_extents_execute_in_all_stages() {
    for source in [
        program("vla_block_lifetime").source,
        BACKWARD_GOTO,
        VOLATILE_ESCAPE,
        INNER_LABEL,
        BETWEEN_ARRAYS,
        NESTED_HEADER_CONTINUE,
        NESTED_HEADER_BREAK,
    ] {
        for target in [Target::LinuxGnu, Target::WindowsGnu] {
            for (optimize, profile) in STAGES {
                with_module(source, target, optimize, profile, |module, _| {
                    let arena = Bump::new();
                    let mut host = DefaultHost::for_target(&arena, target);
                    let outcome = interpreter::run(module, "main", &[], &mut host).unwrap();
                    assert_eq!(
                        outcome.termination,
                        Termination::Returned(Some(RuntimeValue::Int(0)))
                    );
                    assert!(
                        outcome.memory_cells < 100_000,
                        "{} cells: {target:?} {optimize}",
                        outcome.memory_cells
                    );
                });
            }
        }
    }
}

#[test]
fn vla_native_arms_match_clang() {
    let Some(target) = host() else { return };
    let toolchain = Toolchain::bundled(target).unwrap();
    let directory = TempDir::new("vla");
    let gcc_available =
        crate::test_support::native::run_native(Command::new("gcc").arg("--version"))
            .is_ok_and(|output| output.status.success());
    for (index, source) in [
        program("vla_extents_and_typedefs").source,
        program("vla_block_lifetime").source,
        program("vla_parameter_bounds").source,
        BACKWARD_GOTO,
        VOLATILE_ESCAPE,
        INNER_LABEL,
        BETWEEN_ARRAYS,
    ]
    .into_iter()
    .chain(STATEMENT_EXPRESSION_JUMPS.iter().copied())
    .chain([ALIGNED_VLAS])
    .chain([NESTED_HEADER_CONTINUE, NESTED_HEADER_BREAK])
    .chain(
        TYPEOF_EVALUATIONS
            .iter()
            .filter_map(|source| (!source.contains("k=0")).then_some(*source)),
    )
    .chain(SWITCH_VLAS.iter().copied())
    .chain(M1_REPRODUCERS.iter().copied())
    .chain(M1_LIFETIMES.iter().copied())
    .chain(M2_PROGRAMS.iter().copied())
    .enumerate()
    {
        // Clang O0 on Win64 retains repeated allocations after an earlier
        // live VLA and overflows its default stack at 8192 elements. Keep
        // the native oracle below that limit; the interpreter test above
        // checks the full-sized case and its bounded storage independently.
        let source = if source == BETWEEN_ARRAYS {
            source.replace("n=8192", "n=128")
        } else {
            source.to_owned()
        };
        let input = directory.join(format!("case-{index}.c"));
        std::fs::write(&input, &source).unwrap();
        // Clang 23.1.1 does not implement GCC's overflow-p builtins. These
        // cases discard the predicate result: an explicit effects control
        // checks the same type-site action with Clang, and GCC checks the
        // original source when available.
        let header_jump = matches!(
            source.as_str(),
            NESTED_HEADER_CONTINUE | NESTED_HEADER_BREAK
        );
        let oracle_input = if header_jump {
            // Bundled Clang O0 on Win64 loops indefinitely on continue in
            // the nested condition. GCC Statement Exprs specifies the outer
            // target. Use an explicit outward goto for Clang and retain the
            // original in every bcc arm and the GCC reference below.
            // <https://gcc.gnu.org/onlinedocs/gcc/Statement-Exprs.html>
            let control = directory.join(format!("clang-header-control-{index}.c"));
            let control_source = if source == NESTED_HEADER_CONTINUE {
                source
                    .replace("continue;0;", "goto outer_step;0;")
                    .replace("return 2;}", "return 2;outer_step:;}")
            } else {
                source
                    .replace("break;0;", "goto outer_exit;0;")
                    .replace("return 1;}}", "return 1;}outer_exit:;}")
            };
            std::fs::write(&control, control_source).unwrap();
            control
        } else if source.contains("__builtin_add_overflow_p") {
            let control = directory.join(format!("clang-control-{index}.c"));
            std::fs::write(
                &control,
                format!("#define __builtin_add_overflow_p(a,b,c) ((void)(c),0)\n{source}"),
            )
            .unwrap();
            control
        } else {
            input.clone()
        };
        for opt in [OptLevel::O0, OptLevel::O2] {
            let options = LlvmOptions {
                target,
                opt: Some(opt),
                disable_llvm_passes: false,
                kind: OutputKind::Executable,
                code: crate::target::CodeOptions::default(),
            };
            let clang = directory.join(format!("clang-{index}-{opt:?}.exe"));
            let output = toolchain
                .command(&oracle_input, &clang, options)
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
            let expected = run_native(&clang);
            assert_eq!(expected.0, 0, "Clang case {index}");
            if source.contains("__builtin_add_overflow_p") || header_jump {
                if gcc_available {
                    let gcc = directory.join(format!("gcc-{index}-{opt:?}.exe"));
                    let output = crate::test_support::native::run_native(
                        Command::new("gcc")
                            .args([
                                "-std=gnu99",
                                if opt == OptLevel::O0 { "-O0" } else { "-O2" },
                            ])
                            .arg(&input)
                            .arg("-o")
                            .arg(&gcc),
                    )
                    .unwrap();
                    assert!(
                        output.status.success(),
                        "{}",
                        String::from_utf8_lossy(&output.stderr)
                    );
                    assert_eq!(run_native(&gcc), expected, "GCC original case {index}");
                } else {
                    eprintln!(
                        "GCC original extension reference unavailable; Clang control and bcc \
                         native arms remain tested"
                    );
                }
            }
            for optimize in [false, true] {
                let executable = directory.join(format!("bcc-{index}-{opt:?}-{optimize}.exe"));
                with_configured_module(
                    &source,
                    target,
                    optimize,
                    Profile::PostAbi,
                    CompilerConfiguration::default()
                        .with_target(target)
                        .with_gnu_extensions(true)
                        .with_extension_policy(crate::configuration::ExtensionPolicy::Allow),
                    |module, _| {
                        compile_module(module, &executable, options).unwrap();
                    },
                );
                assert_eq!(
                    run_native(&executable),
                    expected,
                    "case {index} {opt:?} {optimize}"
                );
            }
        }
    }
}

const M1_REPRODUCERS: &[&str] = &[
    "int main(void){ int n=3, a[n][n], (*p)[n]=a; __typeof__(*({ goto L; L:; p++; })) b; return \
     p!=a+1 || sizeof b!=12; }",
    "int main(void){ int n=2, a[n]; __typeof__(({ goto L; L: &a; })) p = &a; return p != &a; }",
    "int main(void){ int n=2; __typeof__(({ again:; int a[n]; if (n==2){ n=1; goto again; } &a; \
     })) p; return n-1; }",
    "int main(void){ int n=2, a[n][n], (*p)[n]=a; a[0][0]=0; (void)__builtin_add_overflow_p(0, 0, \
     ((__typeof__(p++))a)[0][0]); return p != a+1; }",
    "int f(int n, int (*p)[n], __typeof__((n++,p)) q){ return n!=4; } int main(void){ int \
     a[3][3]; return f(3,a,a); }",
];

#[test]
fn m1_control_and_type_site_regressions() {
    for source in M1_REPRODUCERS {
        for target in [Target::LinuxGnu, Target::WindowsGnu] {
            for (optimize, profile) in STAGES {
                with_module(source, target, optimize, profile, |module, target| {
                    assert_eq!(interpret(module, target).0, 0, "{source}");
                });
            }
        }
    }
}

const M1_LIFETIMES: &[&str] = &[
    "int main(void){int n=2;__typeof__(int[n++]) a;return n!=3 || sizeof a!=8;}",
    "int f(int n,__typeof__(int[n++]) a){return n!=4;}int main(void){int a[3];return f(3,a);}",
    "int main(void){int n=2;(void)(__typeof__(({int a[n];goto L;L:;&a;})))0;return 0;}",
    "int main(void){int n=2;(void)(__typeof__(int (*)[n++]))0;return n!=3;}",
    "int main(void){int n=128,total=0; left:{int a[n]; a[0]=++total; if(total<5)goto right;} goto \
     done; right:{int b[n]; b[0]=++total; if(total<5)goto left;} done:return total!=5;}",
    "int main(void){int n=128,i=0; int a[n]; a[0]=42; again:; typedef int T[n]; T *p=&a; int \
     b[n]; b[0]=++i; if(i<1000)goto again; return a[0]!=42 || p!=&a || b[0]!=1000;}",
    "int main(void){int n=128,i=0; while(i<1000){typedef int T[n]; T *p; int a[n]; p=&a; \
     a[0]=++i; if((*p)[0]!=i)return 1;}return 0;}",
    "struct S{int x;}; int main(void){int n=3; struct S value=({struct S a[n];a[0].x=42;a[0];}); \
     return value.x!=42;}",
    "int main(void){int n=128,i=0,total=0; while(i<1000){int a[n];a[0]=++i;switch(i%3){case 0: \
     switch(i%2){case 0:total++;break;default:total++;}break;case 1 ... \
     2:total++;break;default:return 2;}if(a[0]!=i)return 1;}return total!=1000;}",
    "int main(void){int n=128,total=0;for(int i=0;i<1000;i++){int a[n];a[0]=i;int \
     *p=__builtin_alloca(4);*p=7;total+=a[0]+*p;}return total!=506500;}",
    "int main(void){int n=2,a[n];a[0]=42;int r=sizeof(int[({goto L; L:; a[0]!=42?1:3;})]);return \
     r!=12 || a[0]!=42;}",
    "int main(void){int n=2,a[n];a[0]=42;int x=({goto L;L:;a[0];});return x!=42 || a[0]!=42;}",
    "int main(void){int n=2,a[n];a[0]=42;switch(({goto L;L:;a[0];})){case 42:return \
     a[0]!=42;default:return 1;}}",
    "int main(void){int n=128,total=0;for(int i=0;i<1000;i++){for(int \
     a[n];total<500;){a[0]=++total;break;}for(int b[n];total<1000;){b[0]=++total;}}return \
     total!=1000;}",
    "int f(int n,int (*p)[n++],__typeof__((n++,p)) q,int (*r)[n++]){return n!=6;}int \
     main(void){int a[3][3],b[5][5];return f(3,a,a,b);}",
    "int main(void){int n=2,a[n][n],(*p)[n]=a;a[0][0]=0;(void)__builtin_add_overflow_p(0,0,0 && \
     ((__typeof__(p++))a)[0][0]);return p!=a;}",
];

#[test]
fn m1_transitions_preserve_values_and_bound_stack_growth() {
    for source in M1_LIFETIMES {
        for target in [Target::LinuxGnu, Target::WindowsGnu] {
            for (optimize, profile) in STAGES {
                with_configured_module(
                    source,
                    target,
                    optimize,
                    profile,
                    CompilerConfiguration::default()
                        .with_target(target)
                        .with_gnu_extensions(true)
                        .with_extension_policy(crate::configuration::ExtensionPolicy::Allow),
                    |module, target| {
                        let arena = Bump::new();
                        let mut host = DefaultHost::for_target(&arena, target);
                        let outcome = interpreter::run(module, "main", &[], &mut host).unwrap();
                        assert_eq!(
                            outcome.termination,
                            Termination::Returned(Some(RuntimeValue::Int(0))),
                            "{source}"
                        );
                        assert!(
                            outcome.memory_cells < 100_000,
                            "{} cells: {source}",
                            outcome.memory_cells
                        );
                    },
                );
            }
        }
    }
}

#[test]
fn parameter_addresses_follow_retained_aliases() {
    for source in [
        "int f(int,int*); int f(int n,int a[(*(&n)=5)]){int *p=&n; return *p;} int main(void){int \
         a[5]; return f(3,a)!=5;}",
        "int f(int n,int a[(*(&n)=5)]){return n;} int main(void){int a[5]; return f(3,a)!=5;}",
        "int f(x) int x; { int *p=&x; *p=37; return x; } int main(void){return f(1)!=37;}",
        "int f(int); int f(int x){int *p=&x; *p=37; return x;} int main(void){return f(1)!=37;}",
        "struct S {int x;}; int f(s) struct S s; {struct S *p=&s; p->x=37; return s.x;} int \
         main(void){struct S s={1}; return f(s)!=37 || s.x!=1;}",
        "struct S {int x;}; int f(struct S); int f(struct S s){struct S *p=&s; p->x=37; return \
         s.x;} int main(void){struct S s={1}; return f(s)!=37 || s.x!=1;}",
    ] {
        for target in [Target::LinuxGnu, Target::WindowsGnu, Target::WindowsMsvc] {
            for (optimize, profile) in STAGES {
                with_module(source, target, optimize, profile, |module, target| {
                    assert_eq!(interpret(module, target).0, 0);
                });
            }
        }
    }
}

const TYPEOF_EVALUATIONS: &[&str] = &[
    "int main(void){int n=3,a[n][n],(*p)[n]=a; int s=sizeof(__typeof__(p++)); return p!=a || \
     s!=sizeof(void*);}",
    "int main(void){int n=3,a[n][n],(*p)[n]=a; __typeof__(*p++) b; return p!=a+1 || sizeof b!=12;}",
    "int main(void){int n=3,a[n][n],(*p)[n]=a; typedef __typeof__(*p++) Row; Row b; return p!=a+1 \
     || sizeof b!=12;}",
    "int main(void){int n=3,a[n][n],(*p)[n]=a; int (*q)[n]=(__typeof__(p++))a; return p!=a+1 || \
     q!=a;}",
    "int main(void){int n=3,a[n][n],(*p)[n]=a; int s=sizeof(__typeof__(*p++)); return p!=a+1 || \
     s!=12;}",
    "int main(void){int n=3,a[n][n],(*p)[n]=a,k=0; __typeof__(*(k=1,p++)) b[(k=k+1,2)]; return \
     p!=a+1 || k!=2 || sizeof b!=24;}",
    "int main(void){int n=3,a[n][n],(*p)[n]=a; __typeof__(__typeof__(*p++)) b; return p!=a+1 || \
     sizeof b!=12;}",
    "int main(void){int x=1; __typeof__(x++) y=2; return x!=1 || y!=2;}",
];

#[test]
fn typeof_vm_operands_execute_at_their_type_site() {
    for source in TYPEOF_EVALUATIONS {
        for target in [Target::LinuxGnu, Target::WindowsGnu] {
            for (optimize, profile) in STAGES {
                with_module(source, target, optimize, profile, |module, target| {
                    assert_eq!(interpret(module, target).0, 0, "{source}");
                });
            }
        }
    }
}

/// Compound literal type names evaluate their typeof operands and bounds
/// when the literal is evaluated (gcc.dg/vla-16.c), and a parameter's
/// function result bounds are evaluated on entry (gcc.dg/pr108423-*.c).
/// C99: §6.7.5.2p5, p. 117; PDF p. 129; §6.9.1p10, p. 142; PDF p. 154.
const WRITTEN_SITE_BOUNDS: &[&str] = &[
    "int a[1]; int main(void){int i=0; int (**p)[1]=&(__typeof__(++i,(int (*)[i])a)){&a}; return \
     *p!=&a || i!=1;}",
    "int a[1]; int main(void){int i=0; (__typeof__(++i,(int (*)[i])a)){&a}++; return i!=1;}",
    "int a[1]; int main(void){int i=0; --(__typeof__(++i,(int (*)[i])a)){&a+1}; return i!=1;}",
    "int a[2][3]; int main(void){int n=3; int (*p)[n]=(int (*)[n++]){a}+1; return p!=a+1 || n!=4;}",
    "int b[2][3]; int (*g(void))[3]{return b;} int f(int n,int (*a(void))[n++]){return n!=4 || \
     sizeof *a()!=12 || a()+1!=b+1;} int main(void){return f(3,g);}",
    "int b[2][3]; int (*g(void))[3]{return b;} int f(int n,int (*(*h)(void))[n]){return sizeof \
     *(*h)()!=12 || (*h)()+1!=b+1;} int main(void){return f(3,&g);}",
];

#[test]
fn compound_literal_and_function_result_bounds_execute() {
    for source in WRITTEN_SITE_BOUNDS {
        for target in [Target::LinuxGnu, Target::WindowsGnu] {
            for (optimize, profile) in STAGES {
                with_module(source, target, optimize, profile, |module, target| {
                    assert_eq!(interpret(module, target).0, 0, "{source}");
                });
            }
        }
    }
}

#[test]
fn vla_restore_releases_newer_alloca_storage() {
    let source = "int main(void){int n=3,*p; {int a[n]; a[0]=11; p=__builtin_alloca(sizeof(int)); \
                  *p=37;} return *p!=37;}";
    for target in [Target::LinuxGnu, Target::WindowsGnu] {
        for (optimize, profile) in STAGES {
            with_module(source, target, optimize, profile, |module, target| {
                let arena = Bump::new();
                let mut host = DefaultHost::for_target(&arena, target);
                let trap = interpreter::run(module, "main", &[], &mut host).unwrap_err();
                assert!(
                    matches!(
                        trap,
                        interpreter::Trap::UndefinedBehavior(
                            interpreter::UbKind::DanglingPointer,
                            _
                        )
                    ),
                    "{trap:?}"
                );
            });
        }
    }
}

const SWITCH_VLAS: &[&str] = &[
    "int main(void){int n=8192; for(int i=0;i<1000;i++){int a[n]; a[0]=i; a[n-1]=37; \
     switch(i%3){case 0: break; case 1: a[0]+=1; break; default: a[0]+=2;} if(a[n-1]!=37 || \
     a[0]!=i+i%3)return 1;} return 0;}",
    "int main(void){int n=8192,total=0; for(int i=0;i<1000;i++){int a[n]; a[0]=i; a[n-1]=37; \
     for(int j=0;j<3;j++){switch(j){case 0: if(a[0]!=i || a[n-1]!=37)return 1; continue; default: \
     break;} if(a[0]!=i || a[n-1]!=37)return 2; total++;} if(a[n-1]!=37)return 3;} return \
     total!=2000;}",
];

#[test]
fn switches_preserve_enclosing_vlas_and_bound_stack_growth() {
    for source in SWITCH_VLAS {
        for target in [Target::LinuxGnu, Target::WindowsGnu] {
            for (optimize, profile) in STAGES {
                with_module(source, target, optimize, profile, |module, target| {
                    let arena = Bump::new();
                    let mut host = DefaultHost::for_target(&arena, target);
                    let outcome = interpreter::run(module, "main", &[], &mut host).unwrap();
                    assert_eq!(
                        outcome.termination,
                        Termination::Returned(Some(RuntimeValue::Int(0)))
                    );
                    assert!(
                        outcome.memory_cells < 100_000,
                        "{} cells",
                        outcome.memory_cells
                    );
                });
            }
        }
    }
}
