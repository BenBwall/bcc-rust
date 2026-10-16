//! GNU layout attribute typing and constant-query regressions.

use super::*;
use crate::{
    configuration::CompilerConfiguration,
    target::Target,
};

#[test]
fn packed_members_and_alignment_have_retained_layouts() {
    let source = r"
struct __attribute__((packed)) P { char c; int n; };
struct M { char c; int n __attribute__((packed, aligned(2))); };
struct __attribute__((packed, aligned(8))) A { char c; int n; };
struct R { _Alignas(16) int values[3]; int __attribute__((aligned(16))) *pointer; };
typedef int small __attribute__((aligned(1)));
int minimum __attribute__((aligned(1)));
small inherited;
int a[__builtin_types_compatible_p(small, int) ? 1 : -1];
int offsets[__builtin_offsetof(struct P,n)==1 && __builtin_offsetof(struct M,n)==2 ? 1 : -1];
int declarations[__alignof__(minimum)==4 && __alignof__(inherited)==1 && __alignof__(((struct R*)0)->values)==16 && __alignof__(((struct R*)0)->values[1])==4 && __alignof__(((struct R*)0)->pointer)==16 && __alignof__(*(((struct R*)0)->pointer))==4 ? 1 : -1];
";
    for target in [Target::LinuxGnu, Target::WindowsGnu] {
        with_configuration(
            source,
            CompilerConfiguration::default()
                .with_target(target)
                .with_gnu_extensions(true),
            |context, sema| {
                assert_eq!(
                    context.error_count(),
                    0,
                    "{:?}",
                    context.take_pending_errors()
                );
                for (name, size, align) in [("P", 5, 1), ("M", 6, 2), ("A", 8, 8)] {
                    let tag = sema
                        .types
                        .tags
                        .iter()
                        .find(|tag| {
                            tag.name
                                .is_some_and(|id| context.string_cache.at(id) == name)
                        })
                        .unwrap();
                    assert_eq!(
                        tag.layout.get(),
                        Some(Layout { size, align }),
                        "{target:?}: {name}"
                    );
                    assert!(!tag.tainted.get());
                }
                let small = sema
                    .bindings
                    .iter()
                    .find(|b| context.string_cache.at(b.name.name) == "small")
                    .unwrap();
                assert_eq!(
                    sema.types.layout(small.ty),
                    Some(Layout { size: 4, align: 1 })
                );
            },
        );
    }
}

#[test]
fn alignas_runtime_array_types_reject_weakened_alignment() {
    for declaration in [
        "_Alignas(1) int a[n];",
        "_Alignas(4) double a[n];",
        "typedef int Row[n]; _Alignas(2) Row a;",
        "typedef double Row[n]; _Alignas(4) Row a;",
        "typedef int Row[n]; _Alignas(1) Row a[2];",
        "_Alignas(1) _Alignas(2) int a[n];",
    ] {
        for target in [
            Target::LinuxGnu,
            Target::LinuxMusl,
            Target::WindowsGnu,
            Target::WindowsMsvc,
        ] {
            let source = format!("void f(int n) {{ {declaration} int following; }}");
            with_configuration(
                &source,
                CompilerConfiguration::default().with_target(target),
                |context, sema| {
                    let errors = context.take_pending_errors();
                    assert_eq!(errors.iter().filter(|error| matches!(error, TranslationError::Semantic(error) if error.kind == SemanticErrorKind::InvalidStandardAlignment)).count(), 1, "{target:?}: {declaration}: {errors:?}");
                    assert!(sema.bindings.iter().any(|binding| {
                        context.string_cache.at(binding.name.name) == "following"
                            && !sema.types.unanalyzed(binding.ty)
                    }));
                },
            );
        }
    }
}

#[test]
fn nested_alignment_operands_retain_their_constant_results() {
    for depth in [32, 1024] {
        let source = format!(
            "int value __attribute__((aligned({}8{}))); int following;",
            "sizeof(int __attribute__((aligned(".repeat(depth),
            "))))".repeat(depth)
        );
        with_source(&source, |context, sema| {
            assert_eq!(
                context.error_count(),
                0,
                "{:?}",
                context.take_pending_errors()
            );
            for name in ["value", "following"] {
                let binding = sema
                    .bindings
                    .iter()
                    .find(|binding| context.string_cache.at(binding.name.name) == name)
                    .unwrap();
                assert_eq!(
                    sema.types.layout(binding.ty),
                    Some(Layout { size: 4, align: 4 })
                );
            }
        });
    }
}

#[test]
fn alignment_constant_expressions_keep_constraint_diagnostics() {
    with_source(
        "[[vendor::aligned(!; arbitrary, {tokens})]] int object; int following;",
        |context, _| {
            assert_eq!(
                context.error_count(),
                0,
                "{:?}",
                context.take_pending_errors()
            );
        },
    );
    for argument in ["sizeof(char)+2", "_Alignof(int)+1", "n"] {
        let source = format!(
            "void f(int n) {{ int a __attribute__((aligned({argument}))); int following; }}"
        );
        with_source(&source, |context, sema| {
            let errors = context.take_pending_errors();
            assert!(errors.iter().any(|error| matches!(error, TranslationError::Semantic(error) if error.kind == SemanticErrorKind::InvalidAlignmentAttribute)), "{argument}: {errors:?}");
            assert!(
                sema.bindings
                    .iter()
                    .any(
                        |binding| context.string_cache.at(binding.name.name) == "following"
                            && !sema.types.unanalyzed(binding.ty)
                    )
            );
        });
    }
    let source = format!(
        "typedef int T; int x __attribute__((aligned({}sizeof(T)*2{}))); int following;",
        "(".repeat(2000),
        ")".repeat(2000)
    );
    with_source(&source, |context, _| {
        assert_eq!(
            context.error_count(),
            0,
            "{:?}",
            context.take_pending_errors()
        );
    });
}

#[test]
fn added_gnu_forms_obey_the_existing_extension_policy() {
    use crate::configuration::{
        CStandard,
        ExtensionPolicy,
    };
    for source in [
        "typedef union {int n;} U; void f(void){U u=(U)1;}",
        "int f(double x){return __builtin_signbit(x);}",
        "int a[3]={[0 ... 2]=1};",
        "typedef int i __attribute__((mode(QI)));",
        "struct __attribute__((packed)) S {char c;int n;};",
    ] {
        for gnu in [false, true] {
            with_configuration(
                source,
                CompilerConfiguration::new(CStandard::C99, ExtensionPolicy::Deny)
                    .with_gnu_extensions(gnu),
                |context, _| {
                    assert!(
                        context.error_count() != 0,
                        "{source} GNU={gnu}: {:?}",
                        context.take_pending_errors()
                    );
                },
            );
        }
    }
}

#[test]
fn scalar_modes_and_standard_alignment_type_complete_declarations() {
    with_configuration(
        r"
typedef signed int __attribute__((mode(QI))) byte;
typedef unsigned int __attribute__((mode(HI))) half;
typedef int __attribute__((mode(DI))) wide;
typedef float __attribute__((mode(DF))) real;
enum E {A,B}; struct S { enum E __attribute__((mode(byte))) e; };
_Alignas(32) int a[3]; _Alignas(0) int b;
int queries[sizeof(byte)==1 && sizeof(half)==2 && sizeof(wide)==8 && sizeof(real)==8 && __alignof__(a)==32 && __alignof__(b)==4 && __builtin_types_compatible_p(__typeof__(((struct S*)0)->e),unsigned char) ? 1 : -1];
",
        CompilerConfiguration::default().with_gnu_extensions(true),
        |context, sema| {
            assert_eq!(
                context.error_count(),
                0,
                "{:?}",
                context.take_pending_errors()
            );
            assert!(
                sema.bindings
                    .iter()
                    .all(|binding| !sema.types.unanalyzed(binding.ty))
            );
        },
    );
}

/// glibc's Clang path spells its complex binary128 type with `mode(TC)`.
#[test]
fn floating_modes_select_real_and_complex_formats_as_clang_does() {
    let source = r"
typedef _Complex float __attribute__((__mode__(__TC__))) tc;
typedef _Complex float __attribute__((mode(KC))) kc;
typedef _Complex float __attribute__((mode(DC))) dc;
typedef _Complex double __attribute__((mode(SC))) sc;
typedef float __attribute__((mode(TF))) tf;
typedef double __attribute__((mode(SF))) sf;
typedef _Complex double __attribute__((mode(SF))) real_of_complex;
int checks[__builtin_types_compatible_p(tc, _Complex __float128)
    && __builtin_types_compatible_p(kc, _Complex __float128)
    && __builtin_types_compatible_p(dc, _Complex double)
    && __builtin_types_compatible_p(sc, _Complex float)
    && __builtin_types_compatible_p(tf, __float128)
    && __builtin_types_compatible_p(sf, float)
    && __builtin_types_compatible_p(real_of_complex, float) ? 1 : -1];
";
    for target in [Target::LinuxGnu, Target::WindowsGnu] {
        with_configuration(
            source,
            CompilerConfiguration::default()
                .with_target(target)
                .with_gnu_extensions(true),
            |context, _| {
                assert_eq!(
                    context.error_count(),
                    0,
                    "{target:?}: {:?}",
                    context.take_pending_errors()
                );
            },
        );
    }
    // A complex mode needs a complex base; MSVC has no binary128 type.
    for (source, target) in [
        (
            "typedef float __attribute__((mode(SC))) bad;",
            Target::LinuxGnu,
        ),
        (
            "typedef _Complex float __attribute__((mode(TC))) bad;",
            Target::WindowsMsvc,
        ),
        (
            "typedef float __attribute__((mode(XF))) bad;",
            Target::WindowsMsvc,
        ),
    ] {
        with_configuration(
            source,
            CompilerConfiguration::default()
                .with_target(target)
                .with_gnu_extensions(true),
            |context, _| {
                let errors = context.take_pending_errors();
                assert!(
                    errors.iter().any(|error| matches!(
                        error,
                        TranslationError::Semantic(error)
                            if error.kind == SemanticErrorKind::InvalidModeAttribute
                    )),
                    "{source}: {errors:?}"
                );
            },
        );
    }
}

/// glibc's Clang path defines C11 `CMPLX` with `__builtin_complex`.
#[test]
fn complex_builtin_types_and_folds_like_clang() {
    with_configuration(
        r"
static const double _Complex z = __builtin_complex(1.0, -0.0);
int checks[__builtin_types_compatible_p(__typeof__(__builtin_complex(1.0f, 2.0f)), float _Complex)
    && __builtin_types_compatible_p(__typeof__(__builtin_complex(1.0L, 2.0L)),
        long double _Complex)
    && __has_builtin(__builtin_complex) ? 1 : -1];
void f(double a, const double b, float c, int d) {
    (void)__builtin_complex(a, b);
    (void)__builtin_complex(c, a);
    (void)__builtin_complex(d, d);
    (void)__builtin_complex(a);
}
int following;
",
        CompilerConfiguration::default().with_gnu_extensions(true),
        |context, sema| {
            let kinds: Vec<_> = context
                .take_pending_errors()
                .iter()
                .filter_map(|error| {
                    if let TranslationError::Semantic(error) = error {
                        Some(error.kind)
                    } else {
                        None
                    }
                })
                .collect();
            assert_eq!(
                kinds,
                [
                    SemanticErrorKind::InvalidArgumentType,
                    SemanticErrorKind::InvalidArgumentType,
                    SemanticErrorKind::InvalidArgumentCount
                ],
            );
            let constant = sema
                .expressions
                .iter()
                .find(|info| {
                    info.constant == ConstantClass::Arithmetic
                        && info.operation_type.is_some()
                        && info.floating.is_some()
                })
                .and_then(|info| info.floating)
                .expect("the static initializer folds");
            assert!(constant.imag.is_zero() && constant.imag.x87_parts().unwrap().0);
        },
    );
}

/// glibc's Clang path routes the C99 classification macros to these.
#[test]
fn floating_classification_builtins_fold_constants_in_each_format() {
    with_configuration(
        r"
int checks[__builtin_isnan(0.0 / 0.0) && !__builtin_isnan(1.0)
    && __builtin_isfinite(1e308) && !__builtin_isfinite(1.0 / 0.0)
    && __builtin_isnormal(1.0) && !__builtin_isnormal(0.0)
    && !__builtin_isnormal(1e-40f) && __builtin_isnormal(1e-40)
    && !__builtin_isnormal(1e-310) && __builtin_isnormal(1e-310L)
    && __builtin_isinf_sign(-1.0 / 0.0) == -1 && __builtin_isinf_sign(1.0f / 0.0f) == 1
    && __builtin_isinf_sign(2.0) == 0
    && __builtin_fpclassify(0, 1, 2, 3, 4, 0.0 / 0.0) == 0
    && __builtin_fpclassify(0, 1, 2, 3, 4, -1.0 / 0.0) == 1
    && __builtin_fpclassify(0, 1, 2, 3, 4, 1.0f) == 2
    && __builtin_fpclassify(0, 1, 2, 3, 4, 1e-40f) == 3
    && __builtin_fpclassify(0, 1, 2, 3, 4, -0.0L) == 4 ? 1 : -1];
int f(int x) { return __builtin_isnan(x) + __builtin_fpclassify(0, 1, 2, 3, 4); }
int following;
",
        CompilerConfiguration::default().with_gnu_extensions(true),
        |context, _| {
            let kinds: Vec<_> = context
                .take_pending_errors()
                .iter()
                .filter_map(|error| {
                    if let TranslationError::Semantic(error) = error {
                        Some(error.kind)
                    } else {
                        None
                    }
                })
                .collect();
            assert_eq!(
                kinds,
                [
                    SemanticErrorKind::InvalidArgumentType,
                    SemanticErrorKind::InvalidArgumentCount
                ],
            );
        },
    );
}

#[test]
fn invalid_representation_attributes_report_constraints_and_recover() {
    with_configuration(
        r"
int a __attribute__((aligned(3)));
typedef float bad __attribute__((mode(QI)));
_Alignas(1) int b;
union U {int i;}; void f(void) { union U u=(union U)1.0; }
int ranges[2] = {[2 ... 1]=1};
int following;
",
        CompilerConfiguration::default().with_gnu_extensions(true),
        |context, sema| {
            let kinds: Vec<_> = context
                .take_pending_errors()
                .iter()
                .filter_map(|error| {
                    if let TranslationError::Semantic(error) = error {
                        Some(error.kind)
                    } else {
                        None
                    }
                })
                .collect();
            for kind in [
                SemanticErrorKind::InvalidAlignmentAttribute,
                SemanticErrorKind::InvalidModeAttribute,
                SemanticErrorKind::InvalidStandardAlignment,
                SemanticErrorKind::InvalidCast,
                SemanticErrorKind::InvalidDesignator,
            ] {
                assert!(kinds.contains(&kind), "{kind:?}: {kinds:?}");
            }
            assert!(
                sema.bindings
                    .iter()
                    .any(
                        |binding| context.string_cache.at(binding.name.name) == "following"
                            && !sema.types.unanalyzed(binding.ty)
                    )
            );
        },
    );
}

#[test]
fn floating_builtins_retain_constants_and_reject_bad_operands() {
    with_configuration(
        "static double c=__builtin_copysign(2.0,-0.0); int a[__builtin_signbit(-0.0)?1:-1]; int \
         f(void){return __builtin_signbit(1)+__builtin_isinf((void*)0)+__builtin_copysign(1.0); } \
         int following;",
        CompilerConfiguration::default().with_gnu_extensions(true),
        |context, sema| {
            let errors = context.take_pending_errors();
            let kinds: Vec<_> = errors
                .iter()
                .filter_map(|error| {
                    if let TranslationError::Semantic(error) = error {
                        Some(error.kind)
                    } else {
                        None
                    }
                })
                .collect();
            assert_eq!(
                kinds
                    .iter()
                    .filter(|&&kind| kind == SemanticErrorKind::InvalidArgumentType)
                    .count(),
                2,
                "{kinds:?}"
            );
            assert!(kinds.contains(&SemanticErrorKind::InvalidArgumentCount));
            let c = sema
                .expressions
                .iter()
                .find(|info| {
                    info.floating.is_some_and(|value| {
                        value
                            .real
                            .compare(crate::float_parsing::LongDouble::from_double(-2.0))
                            == 0
                    })
                })
                .unwrap();
            assert_eq!(c.constant, ConstantClass::Arithmetic);
        },
    );
}

#[test]
fn packed_members_distinguish_typedef_and_explicit_alignment() {
    let source = "typedef int I __attribute__((aligned(4)));
struct __attribute__((packed)) P { char c; I n; };
struct __attribute__((packed)) A { char c; I n __attribute__((aligned(4))); };
struct __attribute__((packed)) B { char c; _Alignas(4) I n; };";
    for target in [Target::LinuxGnu, Target::WindowsGnu, Target::WindowsMsvc] {
        with_configuration(
            source,
            CompilerConfiguration::default().with_target(target),
            |context, sema| {
                assert_eq!(
                    context.error_count(),
                    0,
                    "{:?}",
                    context.take_pending_errors()
                );
                for (name, mut size, mut align, mut offset) in
                    [("P", 5, 1, 1), ("A", 8, 4, 4), ("B", 8, 4, 4)]
                {
                    if target == Target::WindowsMsvc && name == "P" {
                        size = 8;
                        align = 4;
                        offset = 4;
                    }
                    let tag = sema
                        .types
                        .tags
                        .iter()
                        .find(|tag| {
                            tag.name
                                .is_some_and(|id| context.string_cache.at(id) == name)
                        })
                        .unwrap();
                    assert_eq!(
                        tag.layout.get(),
                        Some(Layout { size, align }),
                        "{target:?} {name}"
                    );
                    assert_eq!(tag.members.get()[1].offset, offset);
                }
            },
        );
    }
}

#[test]
fn aggregate_value_layout_excludes_storage_alignment() {
    with_source(
        "struct S {long a,b,c;}; struct S x __attribute__((aligned(64)));",
        |context, sema| {
            assert_eq!(context.error_count(), 0);
            let binding = sema
                .bindings
                .iter()
                .find(|b| context.string_cache.at(b.name.name) == "x")
                .unwrap();
            assert_eq!(
                sema.types.layout(binding.ty),
                Some(Layout {
                    size:  24,
                    align: 64,
                })
            );
            assert_eq!(
                sema.types.aggregate_value_layout(binding.ty),
                Some(Layout {
                    size:  24,
                    align: 8,
                })
            );
            assert_eq!(sema.types.storage_alignment(binding.ty), Some(64));
        },
    );
}

#[test]
fn variable_array_storage_preserves_typedef_alignment_overrides() {
    with_source(
        "void f(int n){typedef int Row[n] __attribute__((aligned(1))); Row a;}",
        |context, sema| {
            assert_eq!(context.error_count(), 0);
            let binding = sema
                .bindings
                .iter()
                .find(|b| context.string_cache.at(b.name.name) == "a")
                .unwrap();
            assert_eq!(sema.types.storage_alignment(binding.ty), Some(1));
            assert_eq!(sema.types.layout(binding.ty), None);
        },
    );
}

#[test]
fn natural_and_member_place_alignment_are_distinct_retained_facts() {
    let source = "typedef int I __attribute__((aligned(1))); struct R { char c; I x; }; struct S \
                  { int x,y; }; struct __attribute__((packed)) P { char c; struct S s; }; int \
                  f(struct P *p) { struct S s=p->s; return s.x; }";
    for target in [Target::LinuxGnu, Target::WindowsGnu, Target::WindowsMsvc] {
        with_configuration(
            source,
            CompilerConfiguration::default().with_target(target),
            |context, sema| {
                assert_eq!(context.error_count(), 0);
                let record = sema
                    .types
                    .tags
                    .iter()
                    .find(|tag| {
                        tag.name
                            .is_some_and(|name| context.string_cache.at(name) == "R")
                    })
                    .unwrap();
                let member = record.members.get()[1];
                assert_eq!(sema.types.layout(member.ty).unwrap().align, 1);
                assert_eq!(sema.types.natural_alignment(member.ty), Some(4));
                assert_eq!(
                    member.offset,
                    if target == Target::LinuxGnu { 1 } else { 4 }
                );
                let access = sema
                    .expressions
                    .iter()
                    .find(|info| {
                        info.place_alignment() == Some(1)
                            && sema.types.tag(info.ty).is_some_and(|tag| {
                                tag.name
                                    .is_some_and(|name| context.string_cache.at(name) == "S")
                            })
                    })
                    .unwrap();
                assert_eq!(
                    sema.types.aggregate_value_layout(access.ty).unwrap().align,
                    4
                );
                assert_eq!(sema.types.natural_alignment(access.ty), Some(4));
                let converted = sema
                    .expression_conversions(access.expression)
                    .last()
                    .unwrap();
                assert_eq!(sema.types.layout(converted.ty).unwrap().align, 4);
                assert_eq!(access.place_alignment(), Some(1));
            },
        );
    }
}

/// Microsoft's `__declspec(align(N))` is Clang's `aligned` attribute: it
/// raises a record's alignment and size, sets a declared object's or
/// typedef's alignment (also below the natural one, as `align(1)` in
/// `TraceLoggingProvider.h` does), and cannot lower a member's. In the
/// declaration specifiers that follow a record body it describes the
/// object, not the record. Values are bundled Clang 23's for
/// `x86_64-pc-windows-msvc -fms-extensions`; MinGW's `__declspec` macro
/// spelling, which Clang ignores, is covered in `targets.rs`.
#[test]
fn declspec_align_is_clangs_aligned_attribute() {
    let source = r#"
struct __declspec(align(16)) A { int x; };
struct B { int x; } const __declspec(align(1)) b1 = {0};
__declspec(align(1)) double d1;
__declspec(align(32)) int i32;
typedef __declspec(align(1)) int T1;
typedef __declspec(align(8)) int T8;
struct M { char c; __declspec(align(8)) int m; };
struct P { char c; __declspec(align(1)) int m; };
const __declspec(allocate(".rdata")) __declspec(align(1)) struct B cb = {1};
__declspec(align) int y;
__declspec(align(1)) __declspec(align(64)) char two;
int checks[_Alignof(struct A) == 16 && sizeof(struct A) == 16 && __alignof__(b1) == 1
    && __alignof__(d1) == 1 && __alignof__(i32) == 32 && _Alignof(T1) == 1 && _Alignof(T8) == 8
    && _Alignof(struct M) == 8 && sizeof(struct M) == 16 && __builtin_offsetof(struct M, m) == 8
    && _Alignof(struct P) == 4 && sizeof(struct P) == 8 && __alignof__(cb) == 1
    && __alignof__(y) == 16 && __alignof__(two) == 64 ? 1 : -1];
"#;
    let configuration = CompilerConfiguration::default()
        .with_target(Target::WindowsMsvc)
        .with_gnu_extensions(true)
        .with_msvc_extensions(true);
    with_configuration(source, configuration, |context, sema| {
        assert_eq!(
            context.error_count(),
            0,
            "{:?}",
            context.take_pending_errors()
        );
        let binding = |name: &str| {
            sema.bindings
                .iter()
                .find(|b| context.string_cache.at(b.name.name) == name)
                .unwrap()
        };
        // The objects have analyzed types, so lowering can emit them.
        for name in ["b1", "d1", "cb", "two"] {
            assert!(!sema.types.unanalyzed(binding(name).ty), "{name}");
        }
    });
    with_configuration(
        "struct S { int a; } __declspec(align(3)) x;",
        CompilerConfiguration::default()
            .with_target(Target::WindowsMsvc)
            .with_msvc_extensions(true),
        |context, _| assert_ne!(context.error_count(), 0),
    );
}

/// Clang combines the declaration's explicit requests before checking C11's
/// original type minimum; Microsoft object requests can replace inherited
/// typedef alignment. C11: §6.7.5p4-6, p. 127; PDF p. 145.
#[test]
fn declared_alignment_combines_explicit_requests_before_checking_the_type() {
    let source = r#"
typedef __declspec(align(8)) int T8;
typedef __declspec(align(8)) long long L8;
__declspec(align(1)) T8 byte;
__declspec(align(1)) T8 suffix __attribute__((aligned(32)));
__declspec(align(64)) T8 stronger __attribute__((aligned(32)));
__declspec(align(1)) __declspec(align(16)) T8 forward;
__declspec(align(16)) __declspec(align(1)) T8 reverse;
__attribute__((aligned(32))) __declspec(align(1)) T8 gnu_first;
__declspec(align(1)) __attribute__((aligned(32))) T8 ms_first;
_Alignas(1) __declspec(align(32)) T8 standard_first;
__declspec(align(32)) _Alignas(1) T8 standard_last;
_Alignas(0) T8 unchanged;
_Alignas(8) __declspec(align(1)) T8 minimum;
_Alignas(1) __attribute__((aligned(32))) T8 gnu;
_Alignas(1) __declspec(align(32)) T8 all __attribute__((aligned(64)));
_Alignas(1) _Alignas(32) __declspec(align(1)) T8 standards;
_Alignas(1) L8 array[2] __attribute__((aligned(32)));
__declspec(align(1)) T8 *pointer __attribute__((aligned(32)));
__declspec(align(32)) T8 function(void);
_Static_assert(_Alignof(T8)==8 && __alignof__(byte)==1, "inherited type");
_Static_assert(__alignof__(suffix)==32 && __alignof__(stronger)==64, "suffix requests");
_Static_assert(__alignof__(forward)==16 && __alignof__(reverse)==16, "MS order");
_Static_assert(__alignof__(gnu_first)==32 && __alignof__(ms_first)==32, "GNU order");
_Static_assert(__alignof__(standard_first)==32 && __alignof__(standard_last)==32, "standard order");
_Static_assert(__alignof__(unchanged)==8 && __alignof__(minimum)==8, "minimum");
_Static_assert(__alignof__(gnu)==32 && __alignof__(all)==64 && __alignof__(standards)==32, "combined");
_Static_assert(__alignof__(array)==32 && __alignof__(pointer)==32, "declarators");
_Static_assert(_Alignof(__typeof__(function()))==8, "return type");
"#;
    for target in [Target::WindowsMsvc, Target::WindowsGnu, Target::LinuxGnu] {
        with_configuration(
            source,
            CompilerConfiguration::default()
                .with_target(target)
                .with_gnu_extensions(true)
                .with_msvc_extensions(true),
            |context, sema| {
                assert_eq!(
                    context.error_count(),
                    0,
                    "{target:?}: {:?}",
                    context.take_pending_errors()
                );
                assert!(
                    sema.bindings
                        .iter()
                        .all(|binding| !sema.types.unanalyzed(binding.ty))
                );
            },
        );
    }
}

#[test]
fn member_alignment_preserves_type_requirements_and_packed_queries() {
    let source = r#"
typedef __declspec(align(8)) int T8;
struct M { char c; __declspec(align(1)) T8 x; } ordinary;
struct I { char c; __declspec(align(1)) int x; } integer;
struct S { char c; _Alignas(1) __declspec(align(32)) T8 x; } standard;
struct A { char c; __declspec(align(1)) T8 x __attribute__((aligned(32))); } suffix;
struct __attribute__((packed)) P { char c; __declspec(align(1)) T8 x; } packed;
struct __attribute__((packed)) Q { char c; T8 x; } inherited;
_Static_assert(_Alignof(struct M)==8 && sizeof(struct M)==16 && __builtin_offsetof(struct M,x)==8 && __alignof__(ordinary.x)==8, "typedef member");
_Static_assert(_Alignof(struct I)==4 && sizeof(struct I)==8 && __builtin_offsetof(struct I,x)==4 && __alignof__(integer.x)==4, "int member");
_Static_assert(_Alignof(struct S)==32 && sizeof(struct S)==64 && __builtin_offsetof(struct S,x)==32 && __alignof__(standard.x)==32, "standard member");
_Static_assert(_Alignof(struct A)==32 && sizeof(struct A)==64 && __builtin_offsetof(struct A,x)==32 && __alignof__(suffix.x)==32, "suffix member");
_Static_assert(__alignof__(packed.x)==1 && __alignof__(inherited.x)==1, "packed expressions");
#ifdef _MSC_VER
_Static_assert(_Alignof(struct P)==8 && sizeof(struct P)==16 && __builtin_offsetof(struct P,x)==8, "MS placement");
_Static_assert(_Alignof(struct Q)==8 && sizeof(struct Q)==16 && __builtin_offsetof(struct Q,x)==8, "MS inherited placement");
#else
_Static_assert(_Alignof(struct P)==1 && sizeof(struct P)==5 && __builtin_offsetof(struct P,x)==1, "GNU placement");
_Static_assert(_Alignof(struct Q)==1 && sizeof(struct Q)==5 && __builtin_offsetof(struct Q,x)==1, "GNU inherited placement");
#endif
"#;
    for target in [Target::WindowsMsvc, Target::WindowsGnu, Target::LinuxGnu] {
        with_configuration(
            source,
            CompilerConfiguration::default()
                .with_target(target)
                .with_gnu_extensions(true)
                .with_msvc_extensions(true),
            |context, _| {
                assert_eq!(
                    context.error_count(),
                    0,
                    "{target:?}: {:?}",
                    context.take_pending_errors()
                )
            },
        );
    }
}

#[test]
fn standard_alignment_checks_the_original_type_even_with_zero_requests() {
    for declaration in [
        "_Alignas(1) __declspec(align(1)) T8 x;",
        "_Alignas(0) __declspec(align(1)) T8 x;",
        "_Alignas(1) T8 x;",
        "_Alignas(1) __declspec(align(1)) int x;",
        "_Alignas(0) __declspec(align(1)) int x;",
        "_Alignas(1) T8 x __attribute__((aligned(1)));",
        "struct M { char c; _Alignas(1) __declspec(align(1)) T8 x; };",
        "struct __attribute__((packed)) M { char c; _Alignas(1) __declspec(align(1)) T8 x; };",
    ] {
        for target in [Target::WindowsMsvc, Target::WindowsGnu, Target::LinuxGnu] {
            let source =
                format!("typedef __declspec(align(8)) int T8; {declaration} int following;");
            with_configuration(
                &source,
                CompilerConfiguration::default()
                    .with_target(target)
                    .with_gnu_extensions(true)
                    .with_msvc_extensions(true),
                |context, sema| {
                    let errors = context.take_pending_errors();
                    assert_eq!(errors.iter().filter(|error| matches!(error, TranslationError::Semantic(error) if error.kind == SemanticErrorKind::InvalidStandardAlignment)).count(), 1, "{target:?}: {declaration}: {errors:?}");
                    assert!(sema.bindings.iter().any(|binding| {
                        context.string_cache.at(binding.name.name) == "following"
                            && !sema.types.unanalyzed(binding.ty)
                    }));
                },
            );
        }
    }
}
