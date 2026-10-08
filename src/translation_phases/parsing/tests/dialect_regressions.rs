//! Regressions for language-mode syntax: GNU `__extension__` scoping, C23
//! storage-class and alignment rules, and the lookahead that separates
//! specifiers, attributes, and labels.

use super::{
    Parsed,
    declaration,
    parser_errors,
    with_parse_configuration,
};
use crate::{
    configuration::{
        CStandard,
        CompilerConfiguration,
        ExtensionPolicy,
    },
    translation_phases::{
        TranslationError,
        parsing::{
            declaration_syntax::TypeSpecifiers,
            errors::ParserErrorType,
            modern::ExtendedType,
            syntax::{
                ExternalDeclaration,
                StorageClass,
            },
        },
        preprocessing::{
            KeywordTokenType,
            TokenType,
        },
    },
};

fn mode(standard: CStandard, gnu: bool, policy: ExtensionPolicy) -> CompilerConfiguration {
    CompilerConfiguration::new(standard, policy).with_gnu_extensions(gnu)
}

fn extensions(parsed: &Parsed<'_, '_>) -> Vec<String> {
    parsed
        .errors
        .iter()
        .filter_map(|error| match error {
            | TranslationError::Extension(extension) => Some(extension.to_string()),
            | _ => None,
        })
        .collect()
}

fn assert_clean_parse(parsed: &Parsed<'_, '_>, source: &str) {
    assert_eq!(
        parser_errors(parsed).count(),
        0,
        "{source}\n{:?}",
        parsed.errors
    );
    assert!(parsed.parser.frames.is_empty(), "{source}");
    assert_eq!(parsed.parser.pedantic_suppression, 0, "{source}");
}

#[test]
fn extension_marker_before_a_member_terminates_and_suppresses_only_that_member() {
    // glibc's `bits/atomic_wide_counter.h` shape.
    let glibc = "typedef union { __extension__ unsigned long long int __value64; struct { \
                 unsigned int __low, __high; } __value32; } W;\nstruct S { __extension__ union { \
                 int a; float b; }; int c; };\n";
    for (standard, gnu) in [
        (CStandard::C89, false),
        (CStandard::C99, false),
        (CStandard::C11, true),
    ] {
        with_parse_configuration(glibc, mode(standard, gnu, ExtensionPolicy::Deny), |p| {
            assert_clean_parse(p, glibc);
            assert_eq!(p.items.len(), 2);
            assert!(extensions(p).is_empty(), "{:?}", p.errors);
        });
    }
    let scoped = "struct S { __extension__ long long a; long long b; };\n";
    with_parse_configuration(
        scoped,
        mode(CStandard::C89, false, ExtensionPolicy::Warn),
        |p| {
            assert_clean_parse(p, scoped);
            let extensions = extensions(p);
            assert_eq!(extensions.len(), 1, "{extensions:?}");
            assert!(extensions[0].contains("long long"));
        },
    );
}

#[test]
fn bracket_after_an_identifier_is_not_an_attribute() {
    let source = "typedef int T;\nstruct S { char T[16]; };\nvoid g(void) { long T[3]; }\n";
    for standard in [CStandard::C99, CStandard::C23] {
        with_parse_configuration(source, mode(standard, false, ExtensionPolicy::Warn), |p| {
            assert_clean_parse(p, source);
        });
    }
    let implicit = "static buf[10];\n";
    with_parse_configuration(
        implicit,
        mode(CStandard::C89, false, ExtensionPolicy::Warn),
        |p| {
            assert_clean_parse(p, implicit);
            assert!(declaration(p, 0).declaration_specifiers.implicit_int);
        },
    );
}

#[test]
fn extension_operand_in_control_headers_is_an_expression() {
    let source = "int f(int x) { if (__extension__ x) return 1; switch (__extension__ x) { \
                  default: break; } for (__extension__ x; x; ) break; if (__extension__ ({ x; })) \
                  x = 2; return 0; }\n";
    with_parse_configuration(
        source,
        mode(CStandard::C17, true, ExtensionPolicy::Warn),
        |p| {
            assert_clean_parse(p, source);
        },
    );
}

#[test]
fn block_scope_function_definition_requires_its_body() {
    let source = "void g(void) {\n int f(void)\n int x;\n x = 0;\n}\nint h(void) { return 0; }\n";
    with_parse_configuration(
        source,
        mode(CStandard::C99, false, ExtensionPolicy::Allow),
        |p| {
            assert_eq!(parser_errors(p).count(), 1, "{:?}", p.errors);
            assert_eq!(p.items.len(), 2);
            assert!(matches!(
                p.items[1],
                ExternalDeclaration::FunctionDefinition(_)
            ));
        },
    );
}

#[test]
fn c23_auto_accompanies_another_storage_class_and_infers_the_type() {
    let source = "static auto a = 3.5;\nauto static b = 3.5;\nextern auto int c;\n";
    with_parse_configuration(
        source,
        mode(CStandard::C23, false, ExtensionPolicy::Warn),
        |p| {
            assert_clean_parse(p, source);
            for item in 0..2 {
                let specifiers = declaration(p, item).declaration_specifiers;
                assert_eq!(specifiers.storage_class, Some(StorageClass::Static));
                assert!(specifiers.auto_with_storage_class);
                assert!(
                    matches!(
                        specifiers.type_specifiers,
                        TypeSpecifiers::Extended(ExtendedType::Inferred)
                    ),
                    "{:?}",
                    specifiers.type_specifiers
                );
            }
            let explicit = declaration(p, 2).declaration_specifiers;
            assert_eq!(explicit.storage_class, Some(StorageClass::Extern));
            assert!(explicit.auto_with_storage_class);
            assert_eq!(explicit.type_specifiers, TypeSpecifiers::Int);
        },
    );
    for (source, standard) in [
        ("typedef auto t = 1;\n", CStandard::C23),
        ("static auto a = 1;\n", CStandard::C17),
        ("auto auto a = 1;\n", CStandard::C23),
    ] {
        with_parse_configuration(source, mode(standard, false, ExtensionPolicy::Warn), |p| {
            assert!(
                parser_errors(p)
                    .any(|error| matches!(error, ParserErrorType::StorageClassRedefinition(..))),
                "{source}: {:?}",
                p.errors
            );
        });
    }
}

#[test]
fn extension_marker_suppression_ends_with_its_parameter() {
    for (source, expected) in [
        ("void f(__extension__ long long a, long long b);\n", 1),
        (
            "void g(void *q) { void (*p)(long long); p = (void (*)(__extension__ long long))q; { \
             long long y; } }\n",
            2,
        ),
        (
            "void h(a, b) __extension__ long long a; long long b; { }\n",
            1,
        ),
    ] {
        with_parse_configuration(
            source,
            mode(CStandard::C89, false, ExtensionPolicy::Warn),
            |p| {
                assert_clean_parse(p, source);
                let extensions = extensions(p);
                assert_eq!(extensions.len(), expected, "{source}: {extensions:?}");
                assert!(extensions.iter().all(|x| x.contains("long long")));
            },
        );
    }
}

#[test]
fn local_label_declaration_is_not_a_statement_before_declarations() {
    let source = "int f(void){ __label__ L; int x = 0; L: return x; }\n";
    with_parse_configuration(
        source,
        mode(CStandard::C89, true, ExtensionPolicy::Warn),
        |p| {
            assert_clean_parse(p, source);
            let extensions = extensions(p);
            assert_eq!(extensions.len(), 1, "{extensions:?}");
            assert!(extensions[0].contains("__label__"));
        },
    );
}

#[test]
fn typedef_name_label_may_follow_another_label() {
    let source = "typedef int T;\nvoid f(void) { a: T: return; }\n";
    with_parse_configuration(
        source,
        mode(CStandard::C99, false, ExtensionPolicy::Warn),
        |p| {
            assert_clean_parse(p, source);
        },
    );
}

#[test]
fn named_jump_needs_its_semicolon_after_the_label() {
    let source = "void f(void){ for(;;){ break\n x = 1; } }\n";
    with_parse_configuration(
        source,
        mode(CStandard::C99, false, ExtensionPolicy::Warn),
        |p| {
            assert!(extensions(p).is_empty(), "{:?}", p.errors);
            let errors: Vec<_> = parser_errors(p).collect();
            assert_eq!(errors.len(), 1, "{errors:?}");
            assert!(matches!(
                errors[0],
                ParserErrorType::ExpectedSemicolonInStatement(
                    "jump statement",
                    Some(TokenType::Identifier)
                )
            ));
        },
    );
    let named = "void f(void){ outer: for(;;){ break outer; continue outer; } }\n";
    with_parse_configuration(
        named,
        mode(CStandard::C2y, false, ExtensionPolicy::Warn),
        |p| {
            assert_clean_parse(p, named);
            assert!(extensions(p).is_empty(), "{:?}", p.errors);
        },
    );
}

#[test]
fn function_specifiers_are_not_compound_literal_storage() {
    for source in [
        "int *p = &(inline int){0};\n",
        "int *p = &(inline const int){0};\n",
    ] {
        with_parse_configuration(
            source,
            mode(CStandard::C23, false, ExtensionPolicy::Warn),
            |p| {
                assert_eq!(
                    parser_errors(p).collect::<Vec<_>>(),
                    [&ParserErrorType::DeclarationSpecifierNotAllowedHere(
                        TokenType::Keyword(KeywordTokenType::Inline)
                    )],
                    "{source}: {:?}",
                    p.errors
                );
            },
        );
    }
    let storage = "int *p = &(static int){0};\n";
    with_parse_configuration(
        storage,
        mode(CStandard::C23, false, ExtensionPolicy::Warn),
        |p| {
            assert_clean_parse(p, storage);
        },
    );
}

#[test]
fn alignment_specifier_belongs_only_to_declarations_and_compound_literals() {
    let source = "int n = sizeof(alignas(8) int);\n";
    for policy in [ExtensionPolicy::Allow, ExtensionPolicy::Warn] {
        with_parse_configuration(source, mode(CStandard::C23, false, policy), |p| {
            assert_eq!(
                parser_errors(p).collect::<Vec<_>>(),
                [&ParserErrorType::DeclarationSpecifierNotAllowedHere(
                    TokenType::Keyword(KeywordTokenType::Alignas)
                )],
                "{:?}",
                p.errors
            );
        });
    }
    let literal = "int *p = &(alignas(8) int){0};\nstruct S { alignas(8) int m; };\n";
    with_parse_configuration(
        literal,
        mode(CStandard::C23, false, ExtensionPolicy::Warn),
        |p| {
            assert_clean_parse(p, literal);
            assert!(extensions(p).is_empty(), "{:?}", p.errors);
        },
    );
}

#[test]
fn implicit_int_after_c89_is_a_removed_feature() {
    let source = "static x = 1;\n";
    for gnu in [false, true] {
        with_parse_configuration(
            source,
            mode(CStandard::C99, gnu, ExtensionPolicy::Warn),
            |p| {
                assert_clean_parse(p, source);
                assert_eq!(
                    extensions(p),
                    ["'implicit int' is a C89 feature removed in C99"]
                );
            },
        );
    }
    with_parse_configuration(
        source,
        mode(CStandard::C89, false, ExtensionPolicy::Warn),
        |p| {
            assert_clean_parse(p, source);
            assert!(extensions(p).is_empty(), "{:?}", p.errors);
        },
    );
}
