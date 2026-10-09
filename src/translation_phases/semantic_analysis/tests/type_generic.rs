//! Atomic, selection and type-generic typing through the parse/analyze seam.

use super::*;
use crate::translation_phases::{
    ErrorSeverity,
    GetSeverity,
};

fn check(source: &str, standard: CStandard, expected: &[SemanticErrorKind]) {
    for gnu in [false, true] {
        with_configuration(
            source,
            crate::configuration::CompilerConfiguration::new(
                standard,
                crate::configuration::ExtensionPolicy::Allow,
            )
            .with_gnu_extensions(gnu),
            |context, unit| {
                let errors = context.take_pending_errors();
                let kinds: Vec<_> = errors
                    .iter()
                    .filter_map(|error| match error {
                        | TranslationError::Semantic(error) => Some(error.kind),
                        | _ => None,
                    })
                    .collect();
                assert_eq!(kinds, expected, "{errors:?}");
                assert_eq!(errors.len(), expected.len(), "{errors:?}");
                // Every invalid case has a valid following declaration.
                assert!(unit.bindings.iter().any(|binding| {
                    context.string_cache.at(binding.name.name) == "following"
                        && unit.types.nodes[binding.ty.index] == TypeKind::Scalar(Scalar::Int)
                }));
            },
        );
    }
}

const TYPEOF_ATOMIC: &str = r#"
_Atomic(int) a;
typeof_unqual(a) b;
typeof_unqual(_Atomic(int)) c;
typeof(a) d;
typeof(_Atomic(int)) e;
_Static_assert(_Generic(&b,int*:1,default:0),"expression");
_Static_assert(_Generic(&c,int*:1,default:0),"type name");
_Static_assert(_Generic(&d,_Atomic(int)*:1,default:0),"typeof expression");
_Static_assert(_Generic(&e,_Atomic(int)*:1,default:0),"typeof type name");
int following;
"#;

#[test]
fn typeof_unqual_removes_atomicity() {
    check(TYPEOF_ATOMIC, CStandard::C23, &[]);
}

const ARRAY_QUALIFIERS: &str = r#"
_Static_assert(__builtin_types_compatible_p(const int[3],int[3]),"array");
_Static_assert(__builtin_types_compatible_p(const volatile int[2][3],int[2][3]),"nested");
_Static_assert(!__builtin_types_compatible_p(const int*[3],int*[3]),"pointee");
_Static_assert(__builtin_types_compatible_p(int *const[3],int*[3]),"pointer qualifiers");
_Static_assert(!__builtin_types_compatible_p(_Atomic(int),int),"atomic identity");
const int a[3]; typeof_unqual(a) b;
const volatile int matrix[2][3]; typeof_unqual(matrix) copy;
typeof_unqual(const int[3]) type_copy;
const int *const pointers[3]; typeof_unqual(pointers) pointer_copy;
_Static_assert(_Generic(&pointer_copy[0],const int**:1,default:0),"pointee retained");
void f(void) { b[0]=1; copy[0][0]=2; type_copy[0]=3; pointer_copy[0]=a; }
int following;
"#;

#[test]
fn array_qualification_stops_at_pointer_targets() {
    check(ARRAY_QUALIFIERS, CStandard::C23, &[]);
}

const CHOOSE_RECOVERY: &str = r#"
void f(void) { int x=__builtin_choose_expr(missing,1,2); int following; }
_Static_assert(__builtin_choose_expr(1,3,4)==3,"valid choice");
"#;

#[test]
fn failed_choose_condition_has_no_dependent_error() {
    check(
        CHOOSE_RECOVERY,
        CStandard::C11,
        &[SemanticErrorKind::UndeclaredIdentifier],
    );
}

const ATOMIC_VM_JUMP: &str = r"
void f(int n) { goto L; _Atomic(int (*)[n]) p; L: ; int following; }
void g(void) { goto M; _Atomic(int (*)[3]) p; M: ; }
";
const ATOMIC_VM_GENERIC: &str = r"
void f(int n) { typedef int A[n]; typedef _Atomic(A*) P;
(void)_Generic(0,P:1,default:0); int following; }
void g(void) { typedef int A[3]; typedef _Atomic(A*) P;
(void)_Generic(0,P:1,default:0); }
";

#[test]
fn atomic_derivation_remains_variably_modified() {
    check(
        ATOMIC_VM_JUMP,
        CStandard::C11,
        &[SemanticErrorKind::JumpIntoVariableScope],
    );
    check(
        ATOMIC_VM_GENERIC,
        CStandard::C11,
        &[SemanticErrorKind::InvalidGenericSelection],
    );
}

const VECTOR_LANE_SELECTION: &str = r"
typedef int V __attribute__((vector_size(16)));
int *f(V v) { return &_Generic(1,int:v[0]); }
int *g(V v) { return &__builtin_choose_expr(1,v[0],0); }
int *h(V v) { return &(_Generic(1,int:__builtin_choose_expr(0,0,(v[0])))); }
int *control(V v, int *p) {
_Generic(1,int:v[0])=1; __builtin_choose_expr(1,v[0],0)=2;
return &_Generic(1,int:p[0]); }
int following;
";

#[test]
fn selected_vector_lanes_cannot_have_addresses() {
    check(
        VECTOR_LANE_SELECTION,
        CStandard::C11,
        &[
            SemanticErrorKind::InvalidVectorOperand,
            SemanticErrorKind::InvalidVectorOperand,
            SemanticErrorKind::InvalidVectorOperand,
        ],
    );
}

const GENERIC_ADDRESSES: &str = r#"
int a[2];
_Static_assert(&a[1]-&a[0]==1,"unwrapped control");
_Static_assert(_Generic(1,int:&a[1]-&a[0])==1,"selected difference");
_Static_assert(_Generic(1,int:&a[1])-&a[0]==1,"generic offset");
_Static_assert(__builtin_choose_expr(1,&a[1],&a[0])-&a[0]==1,"choose offset");
_Static_assert(__builtin_choose_expr(1,&a[1]-&a[0],0)==1,"chosen difference");
_Static_assert(_Generic(1,int:__builtin_choose_expr(1,&a[1],&a[0]))-&a[0]==1,"nested");
_Static_assert(__atomic_always_lock_free(8,&_Generic(1,int:*(char*)0)),"null generic");
_Static_assert(__atomic_always_lock_free(8,&__builtin_choose_expr(1,*(char*)0,*(char*)1)),"null choose");
int *p = _Generic(1,int:&a[1]);
int *q = &_Generic(1,int:a[1]);
int following;
"#;

#[test]
fn selected_addresses_keep_their_base_and_offset() {
    check(GENERIC_ADDRESSES, CStandard::C11, &[]);
}

const ATOMIC_BOOL_ADDRESS: &str = r"
int x; int a[2]; void f(void);
_Bool control=&x; _Atomic(_Bool) b=&x;
_Atomic(_Bool) array=a; _Atomic(_Bool) function=f;
_Atomic(_Bool) null=(void*)0;
int following;
";

#[test]
fn atomic_bool_accepts_static_address_initializers() {
    check(ATOMIC_BOOL_ADDRESS, CStandard::C11, &[]);
}

const LOCK_FREE_SIZE: &str = r#"
_Static_assert(!__atomic_always_lock_free(-1,0),"negative size");
_Static_assert(!__atomic_always_lock_free(-2L,0),"negative long size");
_Static_assert(!__atomic_always_lock_free((unsigned long long)-1,0),"large size");
_Static_assert(__atomic_always_lock_free(4,0),"supported size");
_Static_assert(!__atomic_always_lock_free(3,0),"unsupported size");
int following;
"#;

#[test]
fn lock_free_size_is_converted_before_folding() {
    check(LOCK_FREE_SIZE, CStandard::C11, &[]);
}

const COMPONENT_ADDRESSES: &str = r"
_Complex double z; double *r=&__real__ z; double *i=&__imag__ z;
_Complex float w; float *s=&__real__ w; float *t=&__imag__ w;
double real_value; double *u=&__real__ real_value;
int following;
";

#[test]
fn components_preserve_static_address_eligibility() {
    check(COMPONENT_ADDRESSES, CStandard::C11, &[]);
}

const IMAG_REAL_CALL: &str = r"
double f(void); double d=__imag__ f(); double control=__imag__ 1.0;
int following;
";
const IMAG_INTEGER_CALL: &str = r"
int f(void); int d=__imag__ f(); int control=__imag__ 1;
int following;
";
const IMAG_BINARY128_CALL: &str = r"
__float128 f(void); __float128 d=__imag__ f(); __float128 control=__imag__ 1.0q;
int following;
";
const IMAG_INTEGER_ICE: &str = r#"
int f(void); enum E { value=__imag__ f() };
_Static_assert(__imag__ 3==0,"literal ICE"); int following;
"#;

#[test]
fn known_zero_imaginary_values_keep_operand_restrictions() {
    for source in [IMAG_REAL_CALL, IMAG_INTEGER_CALL, IMAG_BINARY128_CALL] {
        check(
            source,
            CStandard::C11,
            &[SemanticErrorKind::NonConstantInitializer],
        );
    }
    check(
        IMAG_INTEGER_ICE,
        CStandard::C11,
        &[SemanticErrorKind::InvalidConstant],
    );
}

const SHADOWED_MATH_BUILTINS: &str = r"
int f(void) {
int (*__builtin_inf)(int)=0;
int (*__builtin_nan)(int)=0;
int (*__builtin_fabsf128)(int,int)=0;
return (__builtin_inf)(1)+__builtin_nan(2)+__builtin_fabsf128(3,4);
}
double control=__builtin_inf();
__float128 absolute=__builtin_fabsf128(-1.0q);
int following;
";

#[test]
fn shadowed_math_names_use_ordinary_call_typing() {
    check(SHADOWED_MATH_BUILTINS, CStandard::C11, &[]);
}

#[test]
fn address_difference_folding_obeys_extension_policy() {
    use crate::configuration::ExtensionPolicy;
    for gnu in [false, true] {
        for policy in [
            ExtensionPolicy::Allow,
            ExtensionPolicy::Warn,
            ExtensionPolicy::Deny,
        ] {
            with_configuration(
                "int a[2]; _Static_assert(_Generic(1,int:&a[1])-&a[0]==1,\"offset\");",
                crate::configuration::CompilerConfiguration::new(CStandard::C11, policy)
                    .with_gnu_extensions(gnu),
                |context, _| {
                    let errors = context.take_pending_errors();
                    assert_eq!(
                        errors.len(),
                        usize::from(policy != ExtensionPolicy::Allow),
                        "{errors:?}"
                    );
                    if let Some(error) = errors.first() {
                        assert!(
                            matches!(error, TranslationError::Extension(_)),
                            "{errors:?}"
                        );
                        assert_eq!(
                            error.severity(),
                            if policy == ExtensionPolicy::Warn {
                                ErrorSeverity::Warning
                            } else {
                                ErrorSeverity::Error
                            }
                        );
                    }
                },
            );
        }
    }
}
