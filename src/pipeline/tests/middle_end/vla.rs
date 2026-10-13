//! C99 array extents, declaration lifetime, and Clang execution comparisons.
use super::*;

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

#[test]
fn vla_stack_storage_is_bounded_and_extents_execute_in_all_stages() {
    for source in [
        program("vla_block_lifetime").source,
        BACKWARD_GOTO,
        VOLATILE_ESCAPE,
        INNER_LABEL,
        BETWEEN_ARRAYS,
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
    .chain(
        TYPEOF_EVALUATIONS
            .iter()
            .filter_map(|source| (!source.contains("k=0")).then_some(*source)),
    )
    .chain(SWITCH_VLAS.iter().copied())
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
        for opt in [OptLevel::O0, OptLevel::O2] {
            let options = LlvmOptions {
                target,
                opt: Some(opt),
                disable_llvm_passes: false,
                kind: OutputKind::Executable,
            };
            let clang = directory.join(format!("clang-{index}-{opt:?}.exe"));
            let output = toolchain.command(&input, &clang, options).output().unwrap();
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
            let expected = run_native(&clang);
            assert_eq!(expected.0, 0, "Clang case {index}");
            for optimize in [false, true] {
                let executable = directory.join(format!("bcc-{index}-{opt:?}-{optimize}.exe"));
                with_module(&source, target, optimize, Profile::PostAbi, |module, _| {
                    compile_module(module, &executable, options).unwrap();
                });
                assert_eq!(
                    run_native(&executable),
                    expected,
                    "case {index} {opt:?} {optimize}"
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
