//! M3 execution equivalence through retained selections, calls, tails and
//! fields.
use super::*;

const PROGRAMS: &[&str] = &[
    "int f(int x){return x+1;}int main(void){int x=0;
     x+=f(1);x+=(f)(1);x+=(__extension__ f)(1);
     x+=_Generic(0,int:f)(1);x+=__builtin_choose_expr(1,f,0)(1);
     x+=(__extension__ (_Generic(0,int:__builtin_choose_expr(1,(f),0))))(1);
     x+=__builtin_popcount(3);x+=(__builtin_popcount)(3);
     x+=(__extension__ __builtin_popcount)(3);
     x+=_Generic(0,int:__builtin_popcount)(3);
     x+=__builtin_choose_expr(1,__builtin_popcount,0)(3);
     x+=(__extension__ (_Generic(0,int:__builtin_choose_expr(1,(__builtin_popcount),0))))(3);
     return x!=24;}",
    "int f(int x){return x+1;}int main(void){int (*p)(int)=f;int x=0;
     x+=(p)(1);x+=(__extension__ p)(1);x+=_Generic(0,int:p)(1);
     x+=__builtin_choose_expr(1,p,f)(1);x+=((int (*)(int))f)(1);
     x+=(*f)(1);x+=(&f)(1);x+=(0,f)(1);x+=(1?f:f)(1);
     x+=({L:f;})(1);return x!=20;}",
    "int f(int n,...){__builtin_va_list a,b;
     (__extension__ (_Generic(0,int:__builtin_choose_expr(1,(__builtin_va_start),0))))(a,n);
     (_Generic(0,int:(__builtin_va_copy)))(b,a);
     int x=__builtin_va_arg(b,int);(__builtin_choose_expr(1,__builtin_va_end,0))(b);
     (__extension__ (__builtin_va_end))(a);return x;}
     int main(void){return f(1,42)!=42;}",
    "struct S{volatile int x;unsigned b:3;};int main(void){struct S s={7,3};int a[2]={9,4};
     int n=0;({});({int unused;});({{n++;}});({(void)n++;});
     int *p=({n++;first:second:a;});
     const struct S *q=&s;int x=(__extension__ (_Generic(0,int:__builtin_choose_expr(1,q->b,0))));
     int *r=&(__extension__ (_Generic(0,int:__builtin_choose_expr(1,n,0))));*r+=1;
     return n!=4 || p!=a || p[1]!=4 || x!=3 || q->x!=7;}",
];

#[test]
fn m3_resolved_expressions_execute_in_all_interpreter_stages() {
    for source in PROGRAMS {
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
fn m3_resolved_expressions_match_native_arms_and_clang() {
    let Some(target) = host() else { return };
    let toolchain = Toolchain::bundled(target).unwrap();
    let directory = TempDir::new("m3-resolved");
    for (index, source) in PROGRAMS.iter().enumerate() {
        for arm in ARMS {
            let executable = directory.join(format!("m3-{index}-{}.exe", arm.name));
            with_module(source, target, arm.bcc, Profile::PostAbi, |module, _| {
                compile_module(
                    module,
                    &executable,
                    LlvmOptions {
                        opt: Some(arm.opt),
                        disable_llvm_passes: arm.disable_llvm_passes,
                        target,
                        kind: OutputKind::Executable,
                        code: crate::target::CodeOptions::default(),
                    },
                )
                .unwrap();
            });
            assert_eq!(run_native(&executable).0, 0, "program {index} {}", arm.name);
        }
        let input = directory.join(format!("m3-{index}.c"));
        std::fs::write(&input, source).unwrap();
        for opt in [OptLevel::O0, OptLevel::O2] {
            let executable = directory.join(format!("m3-{index}-clang-{opt:?}.exe"));
            let output = toolchain
                .command(
                    &input,
                    &executable,
                    LlvmOptions {
                        opt: Some(opt),
                        disable_llvm_passes: false,
                        target,
                        kind: OutputKind::Executable,
                        code: crate::target::CodeOptions::default(),
                    },
                )
                .output()
                .unwrap();
            assert!(output.status.success(), "{output:?}");
            assert_eq!(
                run_native(&executable).0,
                0,
                "program {index} clang {opt:?}"
            );
        }
    }
}
