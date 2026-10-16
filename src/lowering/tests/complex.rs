//! Complex scalar value semantics and ABI transport on all supported targets.
use super::*;
use crate::{
    abi::lower_abi,
    backend::interpreter::{
        DefaultHost,
        Outcome,
        RuntimeValue,
        Termination,
        Trap,
        UbKind,
        run,
    },
    ir::Value,
    optimizer::{
        OptimizerOptions,
        optimize,
    },
};

const PROGRAM: &str = r"
static double _Complex original = __builtin_complex(3., -4.);
static double _Complex array[2] = {2. + 3.i, -5. + 7.i};
static _Bool imaginary_truth = 1.i;
static struct { _Bool truth; } bool_record = { 2.i };
static double _Complex negative_imaginary_zero=__builtin_complex(2.,-0.)+1.;
float crealf(float _Complex); float cimagf(float _Complex);
static float _Complex narrow_step(float _Complex x) {
 return x*__builtin_complex(2.f,0.f)/__builtin_complex(2.f,0.f);
}
static int real_calls, imag_calls;
static double real(void) { ++real_calls; return -0.; }
static double imaginary(void) { ++imag_calls; return 5.; }
static double _Complex change(double _Complex *p) { *p = 99. + 100.i; return 1. + 2.i; }
static double _Complex transform(double _Complex x) { return x * (2. - 1.i) + (1. + 3.i); }
static double _Complex exhausted(double a,double b,double c,double d,double e,double f,double g,double h,...) {
 __builtin_va_list ap; __builtin_va_start(ap,h);
 double _Complex x = __builtin_va_arg(ap,double _Complex);
 double _Complex y = __builtin_va_arg(ap,double _Complex);
 int tail = __builtin_va_arg(ap,int); __builtin_va_end(ap);
 return x+y+(a+b+c+d+e+f+g+h+tail);
}
struct __attribute__((packed)) Packed { char c; volatile double _Complex z; };
struct Wrapped { double _Complex z; int count; };
static struct Wrapped wrap(struct Wrapped x) { x.z *= 1.i; ++x.count; return x; }
int main(void) {
 double _Complex z = __builtin_complex(real(),imaginary());
 if(real_calls!=1 || imag_calls!=1 || !__builtin_signbit(__real__ z)) return 1;
 double *ip = &__imag__ z; *ip=-2.; __real__ z=4.;
 z += original; z -= 1.+2.i; z *= 1.i; z /= 2.;
 if(z != 4.+3.i || !z || !(_Bool)(1.i) || (_Bool)__builtin_complex(0.,-0.)) return 2;
 unsigned long long large=0xffffffffffffffffULL;
 double _Complex converted=large;
 if(__real__ converted!=(double)large || __imag__ converted!=0. || (int)(7.5+100.i)!=7) return 3;
 float _Complex narrow=__builtin_complex(1.25f,-2.5f); double _Complex wide=narrow;
 if(wide!=1.25-2.5i) return 4;
 double _Complex (*fp)(double _Complex)=transform;
 if(fp(array[0])!=8.+7.i) return 5;
 if(exhausted(1,2,3,4,5,6,7,8,2.+3.i,4.+5.i,9)!=51.+8.i) return 6;
 volatile double big=0x1.fffffffffffffp+1021, tiny=0x1p-1020;
 double _Complex value=__builtin_complex(big,big), ratio=value/value;
 if(ratio!=1.) return 7;
 value=__builtin_complex(tiny,tiny); ratio=value/value;
 if(ratio!=1.) return 8;
 int selected=0; z=selected?__builtin_complex(real(),imaginary()):original;
 if(z!=original || real_calls!=1 || imag_calls!=1) return 9;
 struct Packed packed={1,9.-10.i}; double _Complex snapshot=packed.z;
 packed.z=11.+12.i; if(snapshot!=9.-10.i) return 10;
 z=3.+4.i; value=z+change(&z); if(value!=4.+6.i) return 11;
 volatile double zero=0.; z=__builtin_complex(2.,-zero);
 if(!__builtin_signbit(__imag__(z+1.)) || !__builtin_signbit(__imag__(1.+z))) return 12;
 if((z && 1.i)!=1 || (zero || z)!=1) return 13;
 z=selected?1.:original; if(z!=original) return 14;
 z=({ double _Complex t=2.+3.i; t; }); if(z!=2.+3.i) return 15;
 static _Bool local_truth=3.i;
 if(!imaginary_truth || !bool_record.truth || !local_truth) return 16;
 if(__imag__ real()!=0. || real_calls!=2) return 17;
 float (*extract)(float _Complex)=crealf;
 if(narrow_step(narrow)!=narrow || extract(narrow)!=1.25f || cimagf(narrow)!=-2.5f) return 18;
 if(!__builtin_signbit(__imag__ negative_imaginary_zero)) return 19;
 struct Wrapped wrapped={3.+4.i,5}; wrapped=wrap(wrapped);
 if(wrapped.z!=-4.+3.i || wrapped.count!=6) return 20;
 double _Complex *literal=&(double _Complex){1.+2.i};
 if(*literal!=1.+2.i) return 21;
 struct Wrapped cleared={0}; if(cleared.z || cleared.count) return 22;
 return 0;
}
";

#[test]
fn complex_values_execute_before_and_after_abi_and_optimization() {
    for target in [
        Target::LinuxGnu,
        Target::LinuxMusl,
        Target::WindowsGnu,
        Target::WindowsMsvc,
    ] {
        let text =
            lower_for(PROGRAM, target).unwrap_or_else(|errors| panic!("{target:?}: {errors:?}"));
        for stage in 0..3 {
            let arena = Bump::new();
            let scratch = Bump::new();
            let mut module = parse_module(&arena, &text).unwrap();
            if stage == 2 {
                let mut options = OptimizerOptions::default();
                options.verify_each = true;
                _ = optimize(&mut module, &options, &scratch);
            }
            if stage > 0 {
                lower_abi(&mut module, target, &scratch).unwrap();
            }
            let errors = verify_module(&module, module.profile(), &scratch);
            assert!(errors.is_empty(), "{errors:?}");
            let mut host = DefaultHost::for_target(&scratch, target);
            let outcome = run(&module, "main", &[], &mut host);
            assert!(
                matches!(
                    outcome,
                    Ok(Outcome {
                        termination: Termination::Returned(Some(RuntimeValue::Int(0))),
                        ..
                    })
                ),
                "{target:?} stage {stage}: {outcome:?}\n{module}"
            );
        }
    }
}

#[test]
fn complex_x87_retains_scalar_result_identity() {
    let text = lower_for(
        "long double _Complex f(long double _Complex x){return x;} struct S {long double a,b;}; \
         struct S g(struct S x){return x;}",
        Target::LinuxGnu,
    )
    .unwrap();
    let arena = Bump::new();
    let scratch = Bump::new();
    let mut module = parse_module(&arena, &text).unwrap();
    lower_abi(&mut module, Target::LinuxGnu, &scratch).unwrap();
    let text = module.to_string();
    assert!(
        text.contains("@f(ptr registers(f80, f80, offset 16, complex, align 16)"),
        "{text}"
    );
    assert!(text.contains("@g(ptr sret(size 32, align 16)"), "{text}");
    let parsed = parse_module(&arena, &text).unwrap();
    assert_eq!(parsed.to_string(), text);
    assert!(verify_module(&parsed, Profile::PostAbi, &scratch).is_empty());
}

#[test]
fn complex_constant_division_avoids_intermediate_overflow() {
    let source = "static double _Complex z=(0x1p1020+0x1p1020i)/(0x1p1020+0x1p1020i); int \
                  main(void){return z!=1.;}";
    assert_eq!(
        numeric_programs::run_main("folded complex division", source, Target::LinuxGnu),
        0
    );
}

#[test]
fn complex_abi_uses_nominal_alignment() {
    let text = lower_for(
        "typedef _Complex long double Low __attribute__((aligned(8))); Low f(Low x){return x;}",
        Target::LinuxGnu,
    )
    .unwrap();
    let arena = Bump::new();
    let scratch = Bump::new();
    let mut module = parse_module(&arena, &text).unwrap();
    lower_abi(&mut module, Target::LinuxGnu, &scratch).unwrap();
    assert!(verify_module(&module, Profile::PostAbi, &scratch).is_empty());
    assert!(
        module
            .to_string()
            .contains("registers(f80, f80, offset 16, complex, align 16)")
    );
    assert!(module.to_string().contains("byval(size 32, align 16"));
}

#[test]
fn complex_call_copies_keep_nominal_alignment() {
    for (real, slot) in [
        ("float", "stack_slot 8, align 4"),
        ("double", "stack_slot 16, align 8"),
    ] {
        let source = format!(
            "typedef {real} _Complex Low __attribute__((aligned(1))); extern void sink(Low); void \
             forward(Low x){{sink(x);}}"
        );
        let text = lower_for(&source, Target::LinuxGnu).unwrap();
        // There is no return area or local object here: the outgoing scalar
        // copy itself must have a slot with the nominal alignment.
        assert!(text.contains(slot), "{text}");
        let arena = Bump::new();
        let scratch = Bump::new();
        let mut module = parse_module(&arena, &text).unwrap();
        lower_abi(&mut module, Target::LinuxGnu, &scratch).unwrap();
        assert!(verify_module(&module, Profile::PostAbi, &scratch).is_empty());
    }
}

#[test]
fn complex_payloads_preserve_common_record_sizes() {
    assert_eq!(size_of::<ssa::Op<'_>>(), 32);
    assert_eq!(size_of::<(ssa::Op<'_>, Option<Value>)>(), 48);
    assert_eq!(size_of::<InstData>(), 16);
}

#[test]
fn imaginary_real_operand_preserves_call_and_volatile_load() {
    let text = lower(
        "extern double external(void); double f(void){return __imag__ external();} double \
         g(volatile double *p){return __imag__ *p;}",
    )
    .unwrap();
    assert!(text.contains("call @external("), "{text}");
    assert!(text.contains("load.f64 volatile"), "{text}");
    let text = lower(
        "extern int external_int(void); double f(void){return (double)__imag__ external_int();} \
         double g(volatile int *p){return 1.0+__imag__ *p;} double h(void){return __imag__ \
         external_int()?1.:2.;}",
    )
    .unwrap();
    assert_eq!(text.matches("call @external_int(").count(), 2, "{text}");
    assert!(text.contains("load.i32 volatile"), "{text}");
}

#[test]
fn complex_arguments_preserve_meaningful_poison_in_registers_and_memory() {
    for target in [Target::LinuxGnu, Target::WindowsGnu, Target::WindowsMsvc] {
        for real in ["float", "double"] {
            for prefix in [
                "",
                "double a,double b,double c,double d,double e,double f,double g,double h,",
            ] {
                let arguments = if prefix.is_empty() {
                    ""
                } else {
                    "1,2,3,4,5,6,7,8,"
                };
                let source = format!(
                    "static void ignore({prefix}{real} _Complex z){{}} int main(void){{ {real} \
                     _Complex z; __real__ z=1; ignore({arguments}z); return 0; }}"
                );
                let text = lower_for(&source, target).unwrap();
                for abi in [false, true] {
                    let arena = Bump::new();
                    let scratch = Bump::new();
                    let mut module = parse_module(&arena, &text).unwrap();
                    if abi {
                        lower_abi(&mut module, target, &scratch).unwrap();
                    }
                    let mut host = DefaultHost::for_target(&scratch, target);
                    let outcome = run(&module, "main", &[], &mut host);
                    assert!(
                        matches!(outcome, Err(Trap::UndefinedBehavior(UbKind::Noundef, _))),
                        "{target:?} {real} abi={abi}: {outcome:?}\n{module}"
                    );
                }
            }
        }
    }
}

#[test]
fn complex_x87_memory_contract_excludes_padding() {
    for poison in [false, true] {
        for marked in [false, true] {
            let text = format!(
                "function @sink(ptr aggregate(size 32, align 16, complex, [0: f80, 16: f80]){}) \
                 -> i32 internal {{\nblock0(v0: ptr):\n v1 = iconst.i32 0\n return \
                 v1\n}}\nfunction @main() -> i32 external {{\n slot0 = stack_slot 32, align \
                 16\nblock0:\n v0 = stack_addr slot0\n v1 = iconst.i128 0\nstore.i128 v1, v0, \
                 align 16\n v2 = iconst.i64 16\n v3 = ptr_add v0, v2\nstore.i128 v1, v3, align \
                 16\n v4 = poison.i8\n v5 = iconst.i64 {}\nv6 = ptr_add v0, v5\n store.i8 v4, v6, \
                 align 1\nv7 = call @sink(v0)\n return v7\n}}\n",
                if marked { " noundef" } else { "" },
                if poison { 16 } else { 26 },
            );
            for abi in [false, true] {
                let arena = Bump::new();
                let scratch = Bump::new();
                let mut module = parse_module(&arena, &text).unwrap();
                if abi {
                    lower_abi(&mut module, Target::LinuxGnu, &scratch).unwrap();
                }
                let mut host = DefaultHost::new_in(&scratch);
                let outcome = run(&module, "main", &[], &mut host);
                if poison && marked {
                    assert!(
                        matches!(outcome, Err(Trap::UndefinedBehavior(UbKind::Noundef, _))),
                        "{outcome:?}"
                    );
                } else {
                    assert!(
                        matches!(
                            outcome,
                            Ok(Outcome {
                                termination: Termination::Returned(Some(RuntimeValue::Int(0))),
                                ..
                            })
                        ),
                        "{outcome:?}"
                    );
                }
            }
        }
    }
}

#[test]
fn complex_result_pointer_contract_does_not_read_output() {
    for target in [Target::LinuxGnu, Target::WindowsGnu, Target::WindowsMsvc] {
        for poison_pointer in [false, true] {
            let text = format!(
                "function @produce(ptr result(size 16, align 8, complex, [0: f64, 8: f64]) \
                 noundef) internal {{\nblock0(v0: ptr):\n v1 = iconst.i128 0\n store.i128 v1, v0, \
                 align 8\n return\n}}\nfunction @main() -> i32 external {{\n slot0 = stack_slot \
                 16, align 8\nblock0:\n v0 = {}\n call @produce(v0)\n v1 = iconst.i32 0\n return \
                 v1\n}}\n",
                if poison_pointer {
                    "poison.ptr"
                } else {
                    "stack_addr slot0"
                },
            );
            for abi in [false, true] {
                // A register result eliminates the output pointer from
                // the physical signature; only memory transport retains
                // its incoming pointer contract.
                if poison_pointer && abi && target == Target::LinuxGnu {
                    continue;
                }
                let arena = Bump::new();
                let scratch = Bump::new();
                let mut module = parse_module(&arena, &text).unwrap();
                if abi {
                    lower_abi(&mut module, target, &scratch).unwrap();
                }
                assert!(verify_module(&module, module.profile(), &scratch).is_empty());
                let mut host = DefaultHost::for_target(&scratch, target);
                let outcome = run(&module, "main", &[], &mut host);
                if poison_pointer {
                    assert!(
                        matches!(outcome, Err(Trap::UndefinedBehavior(UbKind::Noundef, _))),
                        "{target:?} abi={abi}: {outcome:?}"
                    );
                } else {
                    assert!(
                        matches!(
                            outcome,
                            Ok(Outcome {
                                termination: Termination::Returned(Some(RuntimeValue::Int(0))),
                                ..
                            })
                        ),
                        "{target:?} abi={abi}: {outcome:?}\n{module}"
                    );
                }
            }
        }
    }
}

#[test]
fn complex_register_results_do_not_freeze_meaningful_bytes() {
    for target in [Target::LinuxGnu, Target::WindowsGnu, Target::WindowsMsvc] {
        let text = lower_for(
            "float _Complex f(float _Complex z){return z;} double _Complex g(double _Complex \
             z){return z;}",
            target,
        )
        .unwrap();
        let arena = Bump::new();
        let scratch = Bump::new();
        let mut module = parse_module(&arena, &text).unwrap();
        lower_abi(&mut module, target, &scratch).unwrap();
        let mut llvm = String::new();
        crate::backend::llvm::emit_module(&module, target, &mut llvm).unwrap();
        assert!(!llvm.contains("freeze"), "{target:?}\n{llvm}");
    }
}
