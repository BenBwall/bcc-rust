//! Torture type/layout and static constant regressions through both back ends.

use super::*;

fn with_module<R>(
    source: &str,
    target: Target,
    optimize: bool,
    profile: Profile,
    then: impl FnOnce(&Module<'_>, Target) -> R,
) -> R {
    with_configured_module(
        source,
        target,
        optimize,
        profile,
        CompilerConfiguration::default()
            .with_target(target)
            .with_gnu_extensions(true),
        then,
    )
}

const PROGRAMS: &[(&str, &str)] = &[
    (
        "layout",
        r"
typedef unsigned int byte __attribute__((mode(QI)));
typedef int small __attribute__((aligned(1)));
typedef small row[3];
enum __attribute__((packed)) E { LOW=0,HIGH=255 };
enum __attribute__((packed)) F { NEGATIVE=-1,POSITIVE=127 };
struct __attribute__((packed)) P { char c; int n; row a; };
struct M { char c; int n __attribute__((packed, aligned(2))); };
struct __attribute__((packed, aligned(8))) A { char c; int n; };
struct __attribute__((packed)) N { char c; struct P p; };
struct __attribute__((packed)) Q { char c; unsigned long long n; };
static struct N data = {1,{2,42,{3,4,5}}};
int main(void) {
 _Alignas(32) int aligned[3];
 byte b=257;
 struct Q q={1,~0ULL};
 if(q.c!=1 || q.n!=~0ULL) return 8;
 enum E e=HIGH; enum F f=NEGATIVE;
 if(sizeof(enum E)!=1 || sizeof(enum F)!=1 || e!=255 || f!=-1) return 7;
 if(sizeof(byte)!=1 || b!=1 || sizeof(small)!=4 || __alignof__(row)!=1) return 1;
 if(sizeof(struct P)!=17 || __builtin_offsetof(struct P,n)!=1 || __builtin_offsetof(struct P,a[2])!=13) return 2;
 if(sizeof(struct M)!=6 || sizeof(struct A)!=8 || __alignof__(struct A)!=8) return 3;
 if(!__builtin_types_compatible_p(__typeof__(data.p.a),row)) return 4;
 if(data.p.n!=42 || data.p.a[2]!=5) return 5;
 data.p.n++; data.p.a[1]+=2;
 if(data.p.n!=43 || data.p.a[1]!=6 || (unsigned long long)aligned%32) return 6;
 return 0;
}
",
    ),
    (
        "union_cast",
        r"
typedef union { int n; double d; struct S {int a,b;} s; } U;
static int calls;
int next(void) { calls++; return 7; }
U make(void) { return (U)next(); }
int main(void) {
 U u=(U)3; U d=(U)2.0;
 U s=(U)(struct S){4,5};
 u=make();
 return u.n!=7 || calls!=1 || s.s.a!=4 || s.s.b!=5 || d.d!=2.0;
}
",
    ),
    (
        "static_addresses",
        r#"
struct A {int i,j;}; struct B {struct A *a,*b;}; struct C {struct B *c;struct A *d;};
static struct C e={&(struct B){&(struct A){1,2},&(struct A){3,4}},&(struct A){5,6}};
static int *absolute=(int*)358273621U;
static const char *strings[]={"hello",((void*)0),"world"};
static int values[4]={1,2,3,4};
static int *offset=&values[2], *identity=&*values;
static int *chosen=__builtin_choose_expr(1,&values[3],&values[1]);
int main(void) {
 if((unsigned long long)absolute!=358273621U || strings[1]) return 1;
 if(e.c->a->i!=1 || e.c->a->j!=2 || e.c->b->j!=4 || e.d->j!=6) return 2;
 if(*offset!=3 || identity!=values || *chosen!=4) return 3;
 e.c->a->i=9;
 return e.c->a->i!=9;
}
"#,
    ),
    (
        "range_initializers",
        r"
struct S {int x,y;};
static int a[] = {[2 ... 5]=7,[4]=3,9};
static struct S pairs[4] = {[1 ... 3]={4,5},[2].y=6};
static int *ptrs[3] = {[0 ... 2]=a+3};
static int calls;
int next(void) { return ++calls; }
struct S make(void) { struct S s={++calls,77}; return s; }
int main(void) {
 int b[5] = {[1 ... 4]=next(),[2]=9};
 int c[2][3] = {[0 ... 1][1 ... 2]=next()};
 struct S d[3] = {[0 ... 2].y=next(),[1].x=8};
 struct S e[4] = {[0 ... 3]=make()};
 if(sizeof(a)!=6*sizeof(int) || a[2]!=7 || a[4]!=3 || a[5]!=9) return 1;
 if(pairs[1].x!=4 || pairs[2].y!=6 || pairs[3].y!=5 || *ptrs[2]!=7) return 2;
 if(calls!=4 || b[1]!=1 || b[2]!=9 || b[4]!=1 || c[0][2]!=2 || c[1][1]!=2) return 3;
 for(int i=0;i<4;i++) if(e[i].x!=4 || e[i].y!=77) return 4;
 e[0].x=99; e[0].y=100;
 for(int i=1;i<4;i++) if(e[i].x!=4 || e[i].y!=77) return 5;
 return d[0].x || d[1].x!=8 || d[2].y!=3;
}
",
    ),
    (
        "floating_builtins",
        r#"
static int sign=__builtin_signbit(-0.0);
static double signed_value=__builtin_copysign(2.0,-0.0);
int ice[__builtin_signbit(-0.0) && __builtin_isinf(__builtin_inf()) ? 1 : -1];
static int calls;
double next(void) { calls++; return -0.0; }
int main(void) {
 volatile double positive=0.0,negative=-0.0,nan=__builtin_nan("");
 volatile float f=-0.0f;
 volatile long double l=-0.0L;
 double infinity=__builtin_inf();
 if(!sign || signed_value!=-2.0 || __builtin_signbit(positive) || !__builtin_signbit(negative)) return 1;
 if(!__builtin_signbitf(f) || !__builtin_signbitl(l) || __builtin_isinf(nan) || __builtin_isinf(1.0)) return 2;
 if(!__builtin_isinf(infinity) || !__builtin_isinf(-infinity) || !__builtin_isinff((float)infinity) || !__builtin_isinfl((long double)infinity)) return 3;
 if(__builtin_copysign(1.0,next())!= -1.0 || calls!=1 || !__builtin_signbit(__builtin_copysign(nan,negative))) return 4;
 if(!__builtin_signbitl(__builtin_copysignl(0.0L,l)) || __builtin_copysignf(2.0f,f)!=-2.0f) return 5;
 return 0;
}
"#,
    ),
    (
        "snprintf",
        r#"
int main(void) {
 char b[8]={'x','x','x','x','x','x','x','x'};
 if(__builtin_snprintf(b,4,"%hhd:%u",(int)255,123u)!=6) return 1;
 if(b[0]!='-' || b[1]!='1' || b[2]!=':' || b[3]!=0 || b[4]!='x') return 2;
 if(__builtin_snprintf((void*)0,0,"%s","hello")!=5) return 3;
 if(__builtin_snprintf(b,1,"abcd")!=4 || b[0]) return 4;
 return 0;
}
"#,
    ),
];

#[test]
fn torture_sema_executes_in_all_stages() {
    for &(name, source) in PROGRAMS {
        // x87 runtime arithmetic/transport remains an interpreter limit.
        // The native arms additionally execute all long-double forms.
        let source = if name == "floating_builtins" {
            source
                .replace("long double", "double")
                .replace("0.0L", "0.0")
                .replace("__builtin_signbitl", "__builtin_signbit")
                .replace("__builtin_isinfl", "__builtin_isinf")
                .replace("__builtin_copysignl", "__builtin_copysign")
        } else {
            source.to_owned()
        };
        for target in [Target::LinuxGnu, Target::WindowsGnu] {
            for (optimize, profile) in STAGES {
                with_module(&source, target, optimize, profile, |module, target| {
                    assert_eq!(
                        interpret(module, target).0,
                        0,
                        "{name} {target:?} {optimize} {profile:?}"
                    );
                });
            }
        }
    }
}

#[test]
fn torture_sema_matches_clang_and_native_arms() {
    let Some(target) = host() else { return };
    let directory = TempDir::new("torture-sema");
    let toolchain = Toolchain::bundled(target).unwrap();
    for &(name, source) in PROGRAMS {
        let input = directory.join(format!("{name}.c"));
        let oracle = directory.join(format!("{name}-clang.exe"));
        // Clang rejects nonconstant GNU ranges in code generation. Compare
        // their once-evaluated meaning with an explicit C99 initializer.
        let oracle_source = if name == "range_initializers" {
            source
                .replace(
                    "int b[5] = {[1 ... 4]=next(),[2]=9};",
                    "int v=next(); int b[5]={0,v,9,v,v};",
                )
                .replace(
                    "int c[2][3] = {[0 ... 1][1 ... 2]=next()};",
                    "int w=next(); int c[2][3]={{0,w,w},{0,w,w}};",
                )
                .replace(
                    "struct S d[3] = {[0 ... 2].y=next(),[1].x=8};",
                    "int z=next(); struct S d[3]={{0,z},{8,z},{0,z}};",
                )
                .replace(
                    "struct S e[4] = {[0 ... 3]=make()};",
                    "struct S once=make(); struct S e[4]={once,once,once,once};",
                )
        } else {
            source.to_owned()
        };
        // Clang exposes type-generic isinf, but not GCC's f/l aliases.
        let oracle_source = oracle_source
            .replace("__builtin_isinff(", "__builtin_isinf(")
            .replace("__builtin_isinfl(", "__builtin_isinf(");
        std::fs::write(&input, oracle_source).unwrap();
        let options = LlvmOptions {
            opt: Some(OptLevel::O0),
            disable_llvm_passes: false,
            target,
            kind: OutputKind::Executable,
            code: crate::target::CodeOptions::default(),
        };
        let mut command = toolchain.command(&input, &oracle, options);
        _ = command.args(["-std=gnu11", "-Wno-constant-conversion"]);
        let compiled = crate::test_support::native::run_native(&mut command).unwrap();
        assert!(
            compiled.status.success(),
            "{name}: {}",
            String::from_utf8_lossy(&compiled.stderr)
        );
        let expected = run_native(&oracle);
        assert_eq!(expected.0, 0, "{name} Clang");
        let mut command = toolchain.command(
            &input,
            &oracle,
            LlvmOptions {
                opt: Some(OptLevel::O2),
                ..options
            },
        );
        _ = command.args(["-std=gnu11", "-Wno-constant-conversion"]);
        let compiled = crate::test_support::native::run_native(&mut command).unwrap();
        assert!(
            compiled.status.success(),
            "{name} Clang O2: {}",
            String::from_utf8_lossy(&compiled.stderr)
        );
        assert_eq!(run_native(&oracle), expected, "{name} Clang O2");
        for arm in ARMS {
            let executable = directory.join(format!("{name}-{}.exe", arm.name));
            with_module(source, target, arm.bcc, Profile::PostAbi, |module, _| {
                compile_module(
                    module,
                    &executable,
                    LlvmOptions {
                        opt: Some(arm.opt),
                        disable_llvm_passes: arm.disable_llvm_passes,
                        ..options
                    },
                )
                .unwrap();
            });
            assert_eq!(run_native(&executable), expected, "{name} {}", arm.name);
        }
    }
}

#[test]
fn torture_sema_static_constants_emit_in_both_backends() {
    let source = PROGRAMS[2].1;
    for target in [Target::LinuxGnu, Target::WindowsGnu] {
        with_module(source, target, false, Profile::PostAbi, |module, target| {
            let mut llvm = String::new();
            crate::backend::llvm::emit_module(module, target, &mut llvm).unwrap();
            assert!(llvm.contains("358273621"), "{llvm}");
            assert!(llvm.contains(".compound"), "{llvm}");
        });
    }
}
