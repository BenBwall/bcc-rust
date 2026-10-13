//! Aggregate varargs preserve declared alignment at every interpreter stage.
use super::*;

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
