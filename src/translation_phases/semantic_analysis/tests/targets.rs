//! Target tests also inspect canonical types directly: _Generic is currently
//! syntax-only in sema, so a parsed C probe alone cannot certify its
//! assertions.

use super::*;
use crate::{
    configuration::{
        CStandard,
        CompilerConfiguration,
        ExtensionPolicy,
    },
    target::Target,
};

const TARGETS: [Target; 4] = [
    Target::LinuxGnu,
    Target::LinuxMusl,
    Target::WindowsGnu,
    Target::WindowsMsvc,
];

#[test]
fn target_typedefs_and_bit_offsets_match_clang() {
    for target in TARGETS {
        with_configuration(
            include_str!("../../../../tests/fixtures/targets/conformance.c"),
            CompilerConfiguration::new(CStandard::C11, ExtensionPolicy::Allow).with_target(target),
            |context, unit| {
                use crate::translation_phases::{
                    ErrorSeverity,
                    GetSeverity as _,
                };
                let errors = context.take_pending_errors();
                assert!(
                    !errors.iter().any(|e| e.severity() == ErrorSeverity::Error),
                    "{}: {errors:?}",
                    target.triple()
                );
                let windows = target.layout().ms_bitfields;
                for (name, scalar) in [
                    ("size_t", target.layout().size_t),
                    ("ptrdiff_t", target.layout().ptrdiff_t),
                    ("wchar_t", target.layout().wchar_t),
                    ("intmax_t", target.layout().intmax_t),
                    ("uintmax_t", target.layout().uintmax_t),
                    ("intptr_t", target.layout().ptrdiff_t),
                    ("uintptr_t", target.layout().size_t),
                ] {
                    let binding = unit
                        .bindings
                        .iter()
                        .find(|b| context.string_cache.at(b.name.name) == name)
                        .unwrap();
                    assert_eq!(
                        unit.types.nodes[binding.ty.index],
                        TypeKind::Scalar(scalar),
                        "{} {name}",
                        target.triple()
                    );
                }
                for (bits, signed, unsigned) in [
                    (8, Scalar::SignedChar, Scalar::UnsignedChar),
                    (16, Scalar::Short, Scalar::UnsignedShort),
                    (32, Scalar::Int, Scalar::UnsignedInt),
                    (
                        64,
                        if windows {
                            Scalar::LongLong
                        } else {
                            Scalar::Long
                        },
                        if windows {
                            Scalar::UnsignedLongLong
                        } else {
                            Scalar::UnsignedLong
                        },
                    ),
                ] {
                    for middle in ["", "_least", "_fast"] {
                        for (prefix, scalar) in [("int", signed), ("uint", unsigned)] {
                            let name = format!("{prefix}{middle}{bits}_t");
                            let binding = unit
                                .bindings
                                .iter()
                                .find(|b| context.string_cache.at(b.name.name) == name)
                                .unwrap();
                            assert_eq!(
                                unit.types.nodes[binding.ty.index],
                                TypeKind::Scalar(scalar),
                                "{} {name}",
                                target.triple()
                            );
                        }
                    }
                }
                for (record, field, byte, bit) in [
                    (
                        "A",
                        "b",
                        if windows { 4 } else { 0 },
                        if windows { 0 } else { 3 },
                    ),
                    ("A", "c", if windows { 8 } else { 1 }, 0),
                    ("D", "b", if windows { 4 } else { 1 }, 0),
                    ("E", "b", 0, 4),
                    ("J", "b", 4, 0),
                ] {
                    let tag = unit
                        .types
                        .tags
                        .iter()
                        .find(|t| t.name.is_some_and(|n| context.string_cache.at(n) == record))
                        .unwrap();
                    let member = tag
                        .members
                        .get()
                        .iter()
                        .find(|m| {
                            m.name
                                .is_some_and(|n| context.string_cache.at(n.name) == field)
                        })
                        .unwrap();
                    assert_eq!(
                        (member.offset, member.bit_offset),
                        (byte, bit),
                        "{} {record}.{field}",
                        target.triple()
                    );
                }
            },
        );
    }
}

#[test]
fn integer_and_wide_literal_types_values_and_code_units_use_the_target() {
    for target in TARGETS {
        with_configuration(
            "void f(void) { (void)2147483648; (void)2147483648L; (void)4294967295UL; \
             (void)L'\\xffff'; (void)L\"\\U0001f600\"; }",
            CompilerConfiguration::new(CStandard::C11, ExtensionPolicy::Allow).with_target(target),
            |context, unit| {
                let windows = target.layout().ms_bitfields;
                let literals: Vec<_> = unit
                    .expressions
                    .iter()
                    .filter(|info| {
                        matches!(
                            info.expression.kind,
                            ExpressionType::Constant(Constant::Integer(_))
                        )
                    })
                    .collect();
                assert_eq!(literals.len(), 3);
                for (literal, expected) in literals.into_iter().zip([
                    if windows {
                        Scalar::LongLong
                    } else {
                        Scalar::Long
                    },
                    if windows {
                        Scalar::LongLong
                    } else {
                        Scalar::Long
                    },
                    Scalar::UnsignedLong,
                ]) {
                    assert_eq!(
                        unit.types.nodes[literal.ty.index],
                        TypeKind::Scalar(expected)
                    );
                    let (bits, signed) = target.layout().integer(expected).unwrap();
                    assert_eq!(
                        (
                            literal.integer.unwrap().bits,
                            literal.integer.unwrap().signed
                        ),
                        (bits, signed)
                    );
                }
                let wide = unit
                    .expressions
                    .iter()
                    .find(|info| {
                        matches!(
                            info.expression.kind,
                            ExpressionType::Constant(Constant::Char(_))
                        )
                    })
                    .unwrap();
                assert_eq!(
                    unit.types.nodes[wide.ty.index],
                    TypeKind::Scalar(target.layout().wchar_t)
                );
                assert_eq!(wide.integer.unwrap().value, 65535);
                let string = unit
                    .expressions
                    .iter()
                    .find(|info| matches!(info.expression.kind, ExpressionType::StringLiteral(_)))
                    .unwrap();
                let ExpressionType::StringLiteral(
                    crate::translation_phases::preprocessing::StringTokenType::WideString(id),
                ) = string.expression.kind
                else {
                    panic!("wide string expected")
                };
                let actual: Vec<_> = context.wide_literal_units(id).collect();
                assert_eq!(
                    actual,
                    if windows {
                        vec![0xD83D, 0xDE00]
                    } else {
                        vec![0x1F600]
                    }
                );
                let TypeKind::Array(element, ArrayBound::Constant(count)) =
                    unit.types.nodes[string.ty.index]
                else {
                    panic!("string array expected")
                };
                assert_eq!(
                    unit.types.nodes[element.index],
                    TypeKind::Scalar(target.layout().wchar_t)
                );
                assert_eq!(count, if windows { 3 } else { 2 });
            },
        );
    }
}

#[test]
fn windows_va_list_operands_require_modifiable_lvalues() {
    for target in [Target::WindowsGnu, Target::WindowsMsvc] {
        for expression in [
            "__builtin_va_end((char*)0);",
            "__builtin_va_arg((char*)0,int);",
            "__builtin_va_copy(a,(char*)0);",
        ] {
            with_configuration(
                &format!("void f(void) {{ __builtin_va_list a; {expression} }}"),
                CompilerConfiguration::default().with_target(target),
                |context, _| {
                    assert!(context.take_pending_errors().iter().any(|e| matches!(e, TranslationError::Semantic(e) if e.kind == SemanticErrorKind::InvalidVaList)));
                },
            );
        }
    }
}

#[test]
fn msvc_long_double_literals_and_operations_have_binary64_precision() {
    for target in TARGETS {
        with_configuration(
            "long double literal=0x1.0000000000000002p0L; long double \
             operation=(0x1p53L+1.0L)-0x1p53L;",
            CompilerConfiguration::default().with_target(target),
            |context, unit| {
                assert_eq!(context.pending_error_count(), 0);
                let results: Vec<_> = unit
                    .expressions
                    .iter()
                    .filter(|info| info.floating.is_some())
                    .collect();
                let literal = results[0].floating.unwrap().real;
                let operation = results.last().unwrap().floating.unwrap().real;
                let binary64 = target == Target::WindowsMsvc;
                assert_eq!(
                    format!("{literal}"),
                    if binary64 {
                        "0x1p+0"
                    } else {
                        "0x1.0000000000000002p+0"
                    }
                );
                assert_eq!(
                    literal.compare(crate::float_parsing::LongDouble::from_double(1.0)) == 0,
                    binary64
                );
                assert_eq!(
                    operation.is_zero(),
                    binary64,
                    "{} {operation:?}",
                    target.triple()
                );
            },
        );
    }
}

#[test]
fn msvc_inline_functions_are_discardable_external_definitions() {
    // As in Clang, the Microsoft ABI emits every C inline function as a
    // discardable external definition rather than a C99 inline definition, so
    // the UCRT's `__inline` functions may keep modifiable static locals.
    let source = "static int y; __inline int *f(void) { static int x; return &x; } inline int \
                  g(void) { return y; }";
    for target in TARGETS {
        with_configuration(
            source,
            CompilerConfiguration::new(CStandard::C17, ExtensionPolicy::Allow)
                .with_gnu_extensions(true)
                .with_target(target),
            |context, unit| {
                let errors: Vec<_> = context
                    .take_pending_errors()
                    .into_iter()
                    .map(|error| match error {
                        | TranslationError::Semantic(error) => error.kind,
                        | other => panic!("unexpected {other:?}"),
                    })
                    .collect();
                let msvc = target == Target::WindowsMsvc;
                assert_eq!(
                    errors,
                    if msvc {
                        vec![]
                    } else {
                        vec![
                            SemanticErrorKind::InlineInternalReference,
                            SemanticErrorKind::InlineStaticObject,
                        ]
                    },
                    "{}",
                    target.triple()
                );
                let inline = unit
                    .definitions
                    .iter()
                    .filter(|d| d.kind == functions::DefinitionKind::Inline)
                    .count();
                assert_eq!(inline, if msvc { 0 } else { 2 }, "{}", target.triple());
            },
        );
    }
}
