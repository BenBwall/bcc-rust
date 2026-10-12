//! GCC scalar builtin contracts across interpretation, optimization and LLVM.
//! GNU extension: GCC manuals cited by the implementation owners.

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
        "builtin_binding",
        r"
int f(unsigned x){return 7;}
int main(void){
 if(__builtin_clz(1u)!=31)return 1;
 {int (*__builtin_clz)(unsigned)=f;if((__builtin_clz)(1)!=7)return 2;}
 {int (*__builtin_add_overflow)(unsigned)=f;if(__builtin_add_overflow(1)!=7)return 3;}
 {int (*__builtin_expect)(unsigned)=f;if(__builtin_expect(1)!=7)return 4;}
 {int (*__builtin_strlen)(unsigned)=f;if(__builtin_strlen(1)!=7)return 5;}
 {int (*__builtin_va_end)(unsigned)=f;if(__builtin_va_end(1)!=7)return 6;}
 return 0;
}
",
    ),
    (
        "bit_operations",
        r"
int f(unsigned x) { return __builtin_clz(x); }
int main(void) {
  unsigned x=0x80000000u; unsigned long y=1; unsigned long long z=1ull<<63;
  if(f(x)!=0 || __builtin_ctz(x)!=31 || __builtin_popcount(x)!=1 || __builtin_parity(x)!=1) return 1;
  if(__builtin_clzl(y)!=sizeof(long)*8-1 || __builtin_ctzl(y)!=0 || __builtin_popcountl(y)!=1 || __builtin_parityl(y)!=1) return 2;
  if(__builtin_clzll(z)!=0 || __builtin_ctzll(z)!=63 || __builtin_popcountll(z|7)!=4 || __builtin_parityll(z|7)!=0) return 3;
  if(__builtin_ffs(0)!=0 || __builtin_ffs(-8)!=4 || __builtin_ffsl(0)!=0 || __builtin_ffsl(8)!=4 || __builtin_ffsll(z)!=64) return 4;
  if(__builtin_clrsb(0)!=31 || __builtin_clrsb(-1)!=31 || __builtin_clrsb(-2147483647-1)!=0 || __builtin_clrsbl(1)!=sizeof(long)*8-2 || __builtin_clrsbll(-1)!=63) return 5;
  if(__builtin_bswap16(0x1234)!=0x3412 || __builtin_bswap32(0x12345678u)!=0x78563412u || __builtin_bswap64(0x0123456789abcdefull)!=0xefcdab8967452301ull) return 6;
  return 0;
} ",
    ),
    (
        "hints_queries",
        r"
static int calls; int bump(void) { return ++calls; }
int main(void) {
  char a[16]; void *p=__builtin_assume_aligned(a,16,0);
  if(p!=a || __builtin_expect(bump(),1)!=1 || __builtin_expect_with_probability(bump(),2,0.5)!=2) return 1;
  __builtin_prefetch(a+bump(),0,3);
  if(calls!=3 || __builtin_constant_p(bump()) || !__builtin_constant_p(1+2) || __builtin_constant_p(1.0)) return 2;
  if(__builtin_object_size((bump(),a),0)!=(__SIZE_TYPE__)-1 || calls!=3 || __builtin_object_size(a,2)!=0) return 3;
  if(__builtin_dynamic_object_size((bump(),a),1)!=(__SIZE_TYPE__)-1 || __builtin_dynamic_object_size(a,3)!=0) return 4;
  if(calls!=3) __builtin_trap();
  if(calls!=3) __builtin_unreachable();
  return 0;
} ",
    ),
    (
        "overflow",
        r"
struct Bits { unsigned a:3; signed b:4; };
int main(void) {
  signed char c=0; unsigned char u=0; int r=0; unsigned v=0; long l; unsigned long ul; long long ll; unsigned long long ull;
  if(!__builtin_add_overflow(127,1,&c) || c!=-128 || __builtin_sub_overflow(-128,-1,&c) || c!=-127) return 1;
  if(!__builtin_sub_overflow(0u,1u,&u) || u!=255 || !__builtin_mul_overflow(100,3,&c) || c!=44) return 2;
  if(__builtin_add_overflow(-1,1u,&v) || v || !__builtin_add_overflow(-2,1u,&v) || v!=~0u) return 3;
  if(!__builtin_sadd_overflow(2147483647,1,&r) || r!=(-2147483647-1) || !__builtin_usub_overflow(0,1,&v) || v!=~0u) return 4;
  if(__builtin_saddl_overflow(1,2,&l) || l!=3 || __builtin_uaddl_overflow(1,2,&ul) || ul!=3 || __builtin_smull_overflow(3,4,&l) || l!=12 || __builtin_umull_overflow(3,4,&ul) || ul!=12) return 5;
  if(__builtin_saddll_overflow(1,2,&ll) || ll!=3 || __builtin_uaddll_overflow(1,2,&ull) || ull!=3 || __builtin_smulll_overflow(3,4,&ll) || ll!=12 || __builtin_umulll_overflow(3,4,&ull) || ull!=12) return 6;
  if(!__builtin_mul_overflow_p(100,3,(signed char)0) || __builtin_add_overflow_p(-1,1u,0u) || !__builtin_sub_overflow_p(0u,1u,0u)) return 7;
  struct Bits b={0,0}; if(!__builtin_add_overflow_p(7,1,b.a) || __builtin_add_overflow_p(6,1,b.a) || !__builtin_add_overflow_p(7,1,b.b)) return 8;
  int side=0; if(__builtin_add_overflow_p(1,2,side++) || side!=1 || __builtin_add_overflow_p(1,2,*(int*)0)) return 9;
  if(__builtin_add_overflow_p(1,2,(side++,*(int*)0)) || side!=2) return 13;
  if(__builtin_sub_overflow_p(1,2,side ? (side++,*(int*)0) : (side+=100,0)) || side!=3) return 14;
  if(__builtin_mul_overflow_p(1,2,(0 && side++)) || side!=3) return 15;
  if(__builtin_mul_overflow_p(1,2,(1 && (side++,*(int*)0))) || side!=4) return 16;
  if(__builtin_mul_overflow_p(1,2,({side++;*(int*)0;})) || side!=5) return 17;
  unsigned __int128 hi=(unsigned __int128)1<<127, wide=0;
  __int128 min=(__int128)hi, out=0;
  if(__builtin_add_overflow(hi,hi,&wide)!=1 || wide!=0 || __builtin_sub_overflow(hi,hi,&out) || out!=0) return 10;
  if(__builtin_add_overflow(min,hi,&out) || out || !__builtin_sub_overflow(min,1,&out) || out!=(__int128)(hi-1)) return 11;
  if(!__builtin_mul_overflow(hi,2u,&wide) || wide || __builtin_mul_overflow(min,1,&out) || out!=min || !__builtin_mul_overflow(min,-1,&out) || out!=min) return 12;
  return 0;
} ",
    ),
    (
        "typed_overflow_conversions",
        r"
static int check(unsigned long long wide, unsigned negative, unsigned __int128 long_wide, unsigned __int128 ll_wide) {
  unsigned u; int s; unsigned long ul; long sl; unsigned long long ull; long long sll;
  if(__builtin_uadd_overflow(wide,1u,&u) || u!=1) return 1;
  if(!__builtin_uadd_overflow(-1,1u,&u) || u!=0) return 2;
  if(__builtin_sadd_overflow(negative,1,&s) || s!=0) return 3;
  if(!__builtin_sadd_overflow(wide+2147483647u,1,&s) || s!=(-2147483647-1)) return 4;
  if(!__builtin_usub_overflow(wide,1u,&u) || u!=~0u) return 5;
  if(__builtin_ssub_overflow(negative,1,&s) || s!=-2) return 6;
  if(__builtin_umul_overflow(2u,wide+1,&u) || u!=2) return 7;
  if(__builtin_smul_overflow(2,negative,&s) || s!=-2) return 8;
  if(__builtin_uaddl_overflow(long_wide,1ul,&ul) || ul!=1) return 9;
  if(__builtin_saddl_overflow(~0ul,1l,&sl) || sl!=0) return 10;
  if(!__builtin_usubl_overflow(long_wide,1ul,&ul) || ul!=~0ul) return 11;
  if(__builtin_ssubl_overflow(~0ul,1l,&sl) || sl!=-2) return 12;
  if(__builtin_umull_overflow(2ul,long_wide+1,&ul) || ul!=2) return 13;
  if(__builtin_smull_overflow(2l,~0ul,&sl) || sl!=-2) return 14;
  if(__builtin_uaddll_overflow(ll_wide,1ull,&ull) || ull!=1) return 15;
  if(__builtin_saddll_overflow(~0ull,1ll,&sll) || sll!=0) return 16;
  if(!__builtin_usubll_overflow(ll_wide,1ull,&ull) || ull!=~0ull) return 17;
  if(__builtin_ssubll_overflow(~0ull,1ll,&sll) || sll!=-2) return 18;
  if(__builtin_umulll_overflow(2ull,ll_wide+1,&ull) || ull!=2) return 19;
  if(__builtin_smulll_overflow(2ll,~0ull,&sll) || sll!=-2) return 20;
  return 0;
}
int main(void) {
  unsigned r;
  if(__builtin_uadd_overflow(0x100000000ULL,1u,&r) || r!=1) return 21;
  volatile unsigned long long wide=0x100000000ULL;
  volatile unsigned negative=~0u;
  volatile unsigned __int128 long_wide=(unsigned __int128)1<<(sizeof(long)*8);
  volatile unsigned __int128 ll_wide=(unsigned __int128)1<<64;
  return check(wide,negative,long_wide,ll_wide);
}
",
    ),
    (
        "overflow_boundaries",
        r"
static int check(int a,int b) {
  signed char c;unsigned char u;short s;unsigned short us;volatile int v;
  long long expected=(long long)a+b;
  int bad=__builtin_add_overflow(a,b,&c);if(bad!=(expected<-128 || expected>127) || c!=(signed char)expected)return 1;
  bad=__builtin_add_overflow(a,b,&u);if(bad!=(expected<0 || expected>255) || u!=(unsigned char)expected)return 2;
  expected=(long long)a-b;
  bad=__builtin_sub_overflow(a,b,&s);if(bad!=(expected<-32768 || expected>32767) || s!=(short)expected)return 3;
  bad=__builtin_sub_overflow(a,b,&us);if(bad!=(expected<0 || expected>65535) || us!=(unsigned short)expected)return 4;
  expected=(long long)a*b;
  bad=__builtin_mul_overflow(a,b,&c);if(bad!=(expected<-128 || expected>127) || c!=(signed char)expected)return 5;
  bad=__builtin_mul_overflow(a,b,&u);if(bad!=(expected<0 || expected>255) || u!=(unsigned char)expected)return 6;
  if(__builtin_add_overflow(a,b,&v) || v!=a+b)return 7;
  return 0;
}
int main(void) {int values[]={-32768,-129,-128,-127,-1,0,1,127,128,255,32767};for(unsigned i=0;i<sizeof(values)/sizeof(values[0]);i++)for(unsigned j=0;j<sizeof(values)/sizeof(values[0]);j++)if(check(values[i],values[j]))return 1;return 0;}
",
    ),
    (
        "dynamic_stack",
        r"
static int f(unsigned n) {int *p[3];for(int i=0;i<3;i++) { p[i]=__builtin_alloca(n*sizeof(int)); p[i][n-1]=i+1;} int *q=alloca(n*sizeof(int));q[n-1]=7;return p[0][n-1]+p[1][n-1]+p[2][n-1]+q[n-1];}
int main(void) {for(int i=0;i<30;i++) if(f(i+1)!=13) return 1;return 0;} ",
    ),
    (
        "alloca_shadowing",
        r"
static int alloca(int x) {return x+1;}
static int f(int x) {return x+2;}
int main(void) {if(alloca(1)!=2)return 1;{int (*alloca)(int)=f;if(alloca(1)!=3)return 2;}return 0;}
",
    ),
    (
        "gnu_expressions",
        r"
static int n; double once(double x) {n++;return x;}
int classify(int x) {switch(x) {case -2147483647-1 ... -2: return 1;case -1 ... 1:return 2;case 2 ... 2147483647:return 3;}return 0;}
int main(void) {
  int a=once(0.5) ?: 7; if(a!=0 || n!=1) return 1;
  double b=once(0) ?: 2.5; if(b!=2.5 || n!=2) return 2;
  int result=({int x=3;for(int i=0;i<4;i++) x+=i; x;}); if(result!=9) return 3;
  ({result++;}); if(result!=10) return 4; ({result;})=11; if(result!=11) return 7; int *address=&({result;}); *address=12; if(result!=12) return 8;
  if(classify(-100)!=1 || classify(0)!=2 || classify(2147483647)!=3) return 5;
  int y=1; __builtin_choose_expr(1,y,++y)=4; if(y!=4) return 6; int *q=&__builtin_choose_expr(1,y,++y); *q=5; if(y!=5) return 9;
  return 0;
} ",
    ),
    (
        "mixed_case_ranges",
        r"
static int classify(int x) {
  int result=0;
  switch(x) {
    case 1: result=1;
    case 10 ... 20: result+=10; break;
    case 30: result=30;
    default: result+=100; break;
  }
  return result;
}
int main(void) {
  int inputs[]={1,10,20,9,21,0,30};
  int expected[]={11,10,10,100,100,100,130};
  for(unsigned i=0;i<sizeof(inputs)/sizeof(inputs[0]);i++)
    if(classify(inputs[i])!=expected[i]) return i+1;
  return 0;
}
",
    ),
    (
        "keyword_varargs",
        r"
#include <stdarg.h>
static int next(va_list a) {return va_arg(a,int);}
static int f(int n, ...) {va_list a,b;va_start(a,n);va_copy(b,a);int x=va_arg(a,int);double y=va_arg(a,double);int z=next(b);va_end(a);va_end(b);return x+z+(int)y;}
int main(void) {return f(3,7,3.5)!=17;} ",
    ),
];

#[test]
fn gcc_scalar_builtins_execute_in_all_stages() {
    for &(name, source) in PROGRAMS {
        // Every new instruction also survives the text parser/printer and both
        // verifier profiles; native tests exercise LLVM's module verifier.
        for target in [Target::LinuxGnu, Target::WindowsGnu] {
            for (optimize, profile) in STAGES {
                with_module(source, target, optimize, profile, |module, target| {
                    let text = module.to_string();
                    let arena = Bump::new();
                    let parsed = parse_module(&arena, &text).unwrap();
                    assert_eq!(parsed.to_string(), text);
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
fn overflow_predicate_effect_analysis_has_linear_work() {
    const DEPTH: usize = 2048;
    let source = format!(
        "int main(void){{int x=1,i=0;int r=__builtin_add_overflow_p(1,2,{}++i{});return r||i!=1;}}",
        "x&&(".repeat(DEPTH),
        ")".repeat(DEPTH)
    );
    crate::test_support::OVERFLOW_EFFECT_VISITS.with(|visits| visits.set(0));
    with_module(
        &source,
        Target::LinuxGnu,
        false,
        Profile::PreAbi,
        |module, target| {
            let visits = crate::test_support::OVERFLOW_EFFECT_VISITS.with(std::cell::Cell::get);
            eprintln!("overflow effect analysis: {visits} visits for depth {DEPTH}");
            assert!(
                visits <= 16 * DEPTH,
                "effect analysis visited {visits} expressions for a {DEPTH}-deep chain"
            );
            assert_eq!(interpret(module, target).0, 0);
        },
    );
}

#[test]
fn gcc_scalar_builtins_execute_in_native_arms() {
    let Some(target) = host() else {
        return;
    };
    let directory = TempDir::new("scalar-builtins");
    for &(name, source) in PROGRAMS {
        for arm in ARMS {
            let executable = directory.join(format!("{name}-{}.exe", arm.name));
            with_module(source, target, arm.bcc, Profile::PostAbi, |module, _| {
                compile_module(
                    module,
                    &executable,
                    LlvmOptions {
                        opt: Some(arm.opt),
                        disable_llvm_passes: arm.disable_llvm_passes,
                        target,
                        kind: OutputKind::Executable,
                    },
                )
                .unwrap();
            });
            assert_eq!(run_native(&executable).0, 0, "{name} {}", arm.name);
        }
    }
}

#[test]
fn gcc_scalar_builtin_poison_and_termination_are_observable() {
    for target in [Target::LinuxGnu, Target::WindowsGnu] {
        for optimize in [false, true] {
            for name in ["clz", "ctz"] {
                let source = format!(
                    "int main(void){{unsigned x=0;if(__builtin_{name}(x)) return 1;return 0;}}"
                );
                with_module(
                    &source,
                    target,
                    optimize,
                    Profile::PostAbi,
                    |module, target| {
                        let arena = Bump::new();
                        let mut host = DefaultHost::for_target(&arena, target);
                        _ = interpreter::run(module, "main", &[], &mut host).unwrap_err();
                    },
                );
            }
            with_module(
                "int main(void){__builtin_trap();return 0;}",
                target,
                optimize,
                Profile::PostAbi,
                |module, target| {
                    let arena = Bump::new();
                    let mut host = DefaultHost::for_target(&arena, target);
                    assert_eq!(
                        interpreter::run(module, "main", &[], &mut host)
                            .unwrap()
                            .termination,
                        Termination::Aborted
                    );
                },
            );
            with_module(
                "int main(void){__builtin_unreachable();return 0;}",
                target,
                optimize,
                Profile::PostAbi,
                |module, target| {
                    let arena = Bump::new();
                    let mut host = DefaultHost::for_target(&arena, target);
                    _ = interpreter::run(module, "main", &[], &mut host).unwrap_err();
                },
            );
            with_module(
                "int main(void){volatile int *p=0;return __builtin_add_overflow_p(1,2,*p);}",
                target,
                optimize,
                Profile::PostAbi,
                |module, target| {
                    let arena = Bump::new();
                    let mut host = DefaultHost::for_target(&arena, target);
                    _ = interpreter::run(module, "main", &[], &mut host).unwrap_err();
                },
            );
        }
    }
}

#[test]
fn gcc_scalar_builtins_emit_matching_llvm_intrinsics() {
    let source = "int f(unsigned x){return \
                  __builtin_clz(x)+__builtin_ctz(x)+__builtin_popcount(x)+__builtin_bswap32(x);} \
                  int g(unsigned x,unsigned y,unsigned *p){return \
                  __builtin_add_overflow(x,y,p)+__builtin_mul_overflow(x,y,p);} void *h(unsigned \
                  n){return __builtin_alloca(n);} void t(void){__builtin_trap();}";
    for target in [Target::LinuxGnu, Target::WindowsGnu] {
        with_module(source, target, false, Profile::PostAbi, |module, target| {
            let mut text = String::new();
            crate::backend::llvm::emit_module(module, target, &mut text).unwrap();
            for name in [
                "llvm.ctlz.i32",
                "llvm.cttz.i32",
                "llvm.ctpop.i32",
                "llvm.bswap.i32",
                "llvm.uadd.with.overflow.i128",
                "llvm.umul.with.overflow.i128",
                "llvm.trap",
                "alloca i8",
            ] {
                assert!(text.contains(name), "missing {name}: {text}");
            }
        });
    }
}

/// The same native arms execute under WSL when its Linux runner is available.
#[test]
fn gcc_scalar_builtins_execute_in_linux_native_arms() {
    if !cfg!(windows)
        || !crate::test_support::native::run_native(Command::new("wsl.exe").args(["-e", "true"]))
            .is_ok_and(|o| o.status.success())
    {
        eprintln!("skipping Linux builtin execution: WSL runner unavailable");
        return;
    }
    let target = Target::LinuxGnu;
    let toolchain = Toolchain::bundled(target).unwrap();
    let directory = TempDir::new("scalar-builtins-linux");
    let helper = directory.join("start.c");
    std::fs::write(&helper,r#"
int main(void);
__attribute__((force_align_arg_pointer,noreturn)) void _start(void) {
    long status=main();__asm__ volatile("syscall"::"a"(60),"D"(status):"rcx","r11","memory");__builtin_unreachable();
}
void *memset(void *p,int c,unsigned long n){volatile unsigned char *a=p;while(n--)*a++=c;return p;}
void *memcpy(void *p,const void *q,unsigned long n){volatile unsigned char *a=p;const volatile unsigned char *b=q;while(n--)*a++=*b++;return p;}
"#).unwrap();
    for &(name, source) in PROGRAMS {
        for arm in ARMS {
            let executable = directory.join(format!("{name}-{}.exe", arm.name));
            let object = directory.join("program.o");
            let options = LlvmOptions {
                opt: Some(arm.opt),
                disable_llvm_passes: arm.disable_llvm_passes,
                target,
                kind: OutputKind::Executable,
            };
            with_module(source, target, arm.bcc, Profile::PostAbi, |module, _| {
                compile_module(
                    module,
                    &object,
                    LlvmOptions {
                        kind: OutputKind::Object,
                        ..options
                    },
                )
                .unwrap();
            });
            let mut link = toolchain.command(&object, &executable, options);
            _ = link.args(["-x", "c"]).arg(&helper).args([
                "-nostdlib",
                "-static",
                "-ffreestanding",
                "-fno-stack-protector",
            ]);
            let linked = crate::test_support::native::run_native(&mut link).unwrap();
            assert!(
                linked.status.success(),
                "{name}: {}",
                String::from_utf8_lossy(&linked.stderr)
            );
            let ran = crate::test_support::native::run_native(
                Command::new("wsl.exe")
                    .arg("-e")
                    .arg(format!(
                        "./{}",
                        executable.file_name().unwrap().to_str().unwrap()
                    ))
                    .current_dir(executable.parent().unwrap()),
            )
            .unwrap();
            assert_eq!(ran.status.code(), Some(0), "{name} Linux {}", arm.name);
        }
    }
}
