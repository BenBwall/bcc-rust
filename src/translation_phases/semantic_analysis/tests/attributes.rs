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
