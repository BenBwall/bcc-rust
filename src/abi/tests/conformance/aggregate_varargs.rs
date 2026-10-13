//! Independently compiled aggregate variadic calls in both directions.
use super::*;

#[test]
fn aggregates_cross_variadic_boundaries() {
    let shapes = [
        (
            "struct S { long a,b,c; };",
            "struct S",
            "{7,8,9}",
            "x.a==7 && x.b==8 && x.c==9",
        ),
        (
            "struct __attribute__((packed)) S { char c; int x; };",
            "struct S",
            "{7,123456}",
            "x.c==7 && x.x==123456",
        ),
        (
            "struct __attribute__((aligned(32))) S { long long x[4]; };",
            "struct S",
            "{{7,8,9,11}}",
            "x.x[0]==7 && x.x[3]==11",
        ),
        (
            "struct __attribute__((aligned(64))) S { long long x[4]; };",
            "struct S",
            "{{7,8,9,11}}",
            "x.x[0]==7 && x.x[3]==11",
        ),
        ("struct S { int x; };", "struct S", "{7}", "x.x==7"),
        (
            "struct S { int x; float y; };",
            "struct S",
            "{7,2.5f}",
            "x.x==7 && x.y==2.5f",
        ),
        (
            "struct S { long long x; double y; };",
            "struct S",
            "{7,2.5}",
            "x.x==7 && x.y==2.5",
        ),
        (
            "struct S { double x,y; };",
            "struct S",
            "{1.5,2.5}",
            "x.x==1.5 && x.y==2.5",
        ),
        (
            "struct S { int x[4]; };",
            "struct S",
            "{{1,2,3,4}}",
            "x.x[0]==1 && x.x[3]==4",
        ),
        (
            "struct S { int x[7]; };",
            "struct S",
            "{{1,2,3,4,5,6,7}}",
            "x.x[0]==1 && x.x[6]==7",
        ),
        (
            "union S { double y; long long x; };",
            "union S",
            "{2.5}",
            "x.y==2.5",
        ),
        (
            "struct S { float x[3]; };",
            "struct S",
            "{{1.5f,2.5f,3.5f}}",
            "x.x[0]==1.5f && x.x[2]==3.5f",
        ),
        (
            "struct S { char x[3]; };",
            "struct S",
            "{{1,2,3}}",
            "x.x[0]==1 && x.x[2]==3",
        ),
        ("struct S { char x; };", "struct S", "{7}", "x.x==7"),
        ("struct S { short x; };", "struct S", "{7}", "x.x==7"),
        ("struct S { int x[0]; };", "struct S", "{}", "1"),
    ];
    for target in [Target::WindowsGnu, Target::WindowsMsvc, Target::LinuxGnu] {
        for (index, (decl, ty, init, check)) in shapes.iter().enumerate() {
            let body = format!(
                "int b_take(int n, ...) {{ __builtin_va_list ap,cp; __builtin_va_start(ap,n);
                for(int i=0;i<n;i++) {{ (void)__builtin_va_arg(ap,long long); \
                 (void)__builtin_va_arg(ap,double); }}
                (void)__builtin_va_arg(ap,{ty});
                __builtin_va_copy(cp,ap); {ty} x=__builtin_va_arg(ap,{ty});
                if(!({check})) return 1; x=__builtin_va_arg(cp,{ty}); if(!({check})) return 2;
                int sentinel=__builtin_va_arg(ap,int); __builtin_va_end(ap); __builtin_va_end(cp); \
                 return sentinel!=123;
            }}"
            );
            let calls = format!(
                "{ty} x __attribute__((aligned(64)))={init}; return c_take(0,x,x,123) || \
                 c_take(5,1LL,1.,2LL,2.,3LL,3.,4LL,4.,5LL,5.,x,x,123) || \
                 c_take(7,1LL,1.,2LL,2.,3LL,3.,4LL,4.,5LL,5.,6LL,6.,7LL,7.,x,x,123) || \
                 c_take(8,1LL,1.,2LL,2.,3LL,3.,4LL,4.,5LL,5.,6LL,6.,7LL,7.,8LL,8.,x,x,123);"
            );
            let bcc = format!("{decl}{body} int c_take(int,...); int b_call(void){{{calls}}}");
            let clang = format!(
                "{decl}{} int b_take(int,...); int b_call(void); int main(void){{int v=b_call(); \
                 if(v)return v; {} }}",
                body.replace("b_take", "c_take"),
                calls.replace("c_take", "b_take")
            );
            for optimized in [false, true] {
                let tu = Bump::new();
                let arena = Bump::new();
                let mut module = lower_c(&tu, &arena, &bcc, target);
                let printed = module.to_string();
                assert_verified(
                    &parse_module(&Bump::new(), &printed).unwrap(),
                    Profile::PreAbi,
                    &printed,
                );
                if optimized {
                    _ = optimize(&mut module, &OptimizerOptions::default(), &Bump::new());
                }
                lower_abi(&mut module, target, &Bump::new()).unwrap();
                let source = format!(
                    "{bcc} {} int main(void){{ {} }}",
                    body.replace("b_take", "c_take"),
                    calls.replace("c_take", "b_take")
                );
                for profile in [Profile::PreAbi, Profile::PostAbi] {
                    let arena = Bump::new();
                    let mut interpreted = lower_c(&tu, &arena, &source, target);
                    if optimized {
                        _ = optimize(&mut interpreted, &OptimizerOptions::default(), &Bump::new());
                    }
                    if profile == Profile::PostAbi {
                        lower_abi(&mut interpreted, target, &Bump::new()).unwrap();
                    }
                    assert_verified(&interpreted, profile, &interpreted.to_string());
                    let execution = Bump::new();
                    let mut host =
                        crate::backend::interpreter::DefaultHost::for_target(&execution, target);
                    let outcome =
                        crate::backend::interpreter::run(&interpreted, "main", &[], &mut host)
                            .unwrap();
                    assert_eq!(
                        outcome.termination,
                        crate::backend::interpreter::Termination::Returned(Some(
                            crate::backend::interpreter::RuntimeValue::Int(0)
                        )),
                        "shape {index} {target:?} {optimized} {profile:?}"
                    );
                }
                assert_conforms(
                    &format!("aggregate-varargs-{index}-{optimized}"),
                    target,
                    &module,
                    &clang,
                );
            }
        }
    }
}

#[test]
fn packed_aggregate_varargs_use_memory_and_indirect_slots() {
    let layout = "ptr aggregate(size 5, align 1, unaligned, [0: i8, 1: i32])";
    let ir = format!(
        "function @c_take(i32, ...) -> i32 external
function @b_take(i32, ...) -> i32 external {{
 slot0 = stack_slot 24, align 8
block0(v0: i32):
 v1 = stack_addr slot0
 va_start v1
 v2 = va_arg_aggregate v1 : ({layout})
 v3 = load.i8 v2, align 1
 v4 = iconst.i64 1
 v5 = ptr_add v2, v4
 v6 = load.i32 v5, align 1
 v7 = iconst.i8 7
 v8 = icmp.i8 ne v3, v7
 v9 = iconst.i32 123456
 v10 = icmp.i32 ne v6, v9
 v11 = or.i1 v8, v10
 v12 = zext.i32 v11
 return v12
}}
function @b_call() -> i32 external {{
 slot0 = stack_slot 5, align 1
block0:
 v0 = stack_addr slot0
 v1 = iconst.i8 7
 store.i8 v1, v0, align 1
 v2 = iconst.i64 1
 v3 = ptr_add v0, v2
 v4 = iconst.i32 123456
 store.i32 v4, v3, align 1
 v5 = func_addr @c_take
 v6 = iconst.i32 0
 v7 = call_indirect v5(v6,v0) : (i32, ..., {layout}) -> i32
 return v7
}}"
    );
    let clang = "struct __attribute__((packed)) P { char c; int i; };
int b_take(int,...); int b_call(void); int c_take(int n,...) { __builtin_va_list ap; \
                 __builtin_va_start(ap,n); struct P p=__builtin_va_arg(ap,struct P); \
                 __builtin_va_end(ap); return p.c!=7 || p.i!=123456; }
int main(void) { struct P p={7,123456}; return b_call() || b_take(0,p); }";
    for target in [Target::WindowsGnu, Target::WindowsMsvc, Target::LinuxGnu] {
        for optimized in [false, true] {
            let arena = Bump::new();
            let mut module = parse_module(&arena, &ir).unwrap();
            if optimized {
                _ = optimize(&mut module, &OptimizerOptions::default(), &Bump::new());
            }
            lower_abi(&mut module, target, &Bump::new()).unwrap();
            assert_conforms("packed-aggregate-varargs", target, &module, clang);
        }
    }
}

#[test]
fn mixed_varargs_fall_back_without_consuming_the_other_register_class() {
    let decl = "struct S { long long i; double d; };";
    let body = "int b_take(int gp, ...) { __builtin_va_list ap; __builtin_va_start(ap,gp);
if(gp) { for(int j=1;j<=5;j++) if(__builtin_va_arg(ap,long long)!=j)return 1; }
else { for(int j=1;j<=8;j++) if(__builtin_va_arg(ap,double)!=j)return 2; }
struct S x=__builtin_va_arg(ap,struct S); if(x.i!=7 || x.d!=2.5)return 3;
if(gp) { if(__builtin_va_arg(ap,double)!=12.5)return 4; }
else { if(__builtin_va_arg(ap,int)!=123)return 5; }
__builtin_va_end(ap); return 0; }";
    let calls = "struct S x={7,2.5}; return c_take(0,1.,2.,3.,4.,5.,6.,7.,8.,x,123) || \
                 c_take(1,1LL,2LL,3LL,4LL,5LL,x,12.5);";
    let bcc = format!("{decl}{body} int c_take(int,...); int b_call(void){{{calls}}}");
    let clang = format!(
        "{decl}{} int b_take(int,...); int b_call(void); int main(void){{ if(b_call())return 6; \
         {} }}",
        body.replace("b_take", "c_take"),
        calls.replace("c_take", "b_take")
    );
    for optimized in [false, true] {
        let tu = Bump::new();
        let arena = Bump::new();
        let mut module = lower_c(&tu, &arena, &bcc, Target::LinuxGnu);
        if optimized {
            _ = optimize(&mut module, &OptimizerOptions::default(), &Bump::new());
        }
        lower_abi(&mut module, Target::LinuxGnu, &Bump::new()).unwrap();
        assert_conforms(
            "mixed-varargs-one-class-exhausted",
            Target::LinuxGnu,
            &module,
            &clang,
        );
        // Both definitions are present for interpreter calls in each direction.
        let source = format!(
            "{bcc} {} int main(void) {{ if(b_call())return 6; {} }}",
            body.replace("b_take", "c_take"),
            calls.replace("c_take", "b_take")
        );
        for profile in [Profile::PreAbi, Profile::PostAbi] {
            let arena = Bump::new();
            let mut module = lower_c(&tu, &arena, &source, Target::LinuxGnu);
            if optimized {
                _ = optimize(&mut module, &OptimizerOptions::default(), &Bump::new());
            }
            if profile == Profile::PostAbi {
                lower_abi(&mut module, Target::LinuxGnu, &Bump::new()).unwrap();
            }
            assert_verified(&module, profile, &module.to_string());
            let execution = Bump::new();
            let mut host =
                crate::backend::interpreter::DefaultHost::for_target(&execution, Target::LinuxGnu);
            let outcome =
                crate::backend::interpreter::run(&module, "main", &[], &mut host).unwrap();
            assert_eq!(
                outcome.termination,
                crate::backend::interpreter::Termination::Returned(Some(
                    crate::backend::interpreter::RuntimeValue::Int(0)
                )),
                "{optimized} {profile:?}"
            );
        }
    }
}
