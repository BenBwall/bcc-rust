//! Independent MSVC grammar gates, syntax ownership and recovery.

use super::{
    super::{
        declaration_syntax::{
            DirectDeclarator,
            TypeQualifiers,
            TypeSpecifiers,
        },
        modern::{
            AttributeSpecifier,
            AttributeSyntax,
            ExtendedType,
        },
        msvc::{
            MsAsm,
            Seh,
        },
        syntax::{
            Statement,
            StatementType,
        },
    },
    parser_errors,
    with_parse_configuration,
};
use crate::{
    configuration::{
        CStandard,
        CompilerConfiguration,
        ExtensionPolicy,
        MsvcFeature,
    },
    translation_phases::GetSeverity,
};

const FEATURES: &[(MsvcFeature, &str)] = &[
    (
        MsvcFeature::Declspec,
        "__declspec(dllexport align(16)) int x;\n",
    ),
    (
        MsvcFeature::IntTypes,
        "__int8 a; unsigned __int16 b; __int32 signed c; __int64 d;\n",
    ),
    (
        MsvcFeature::CallingConventions,
        "int __cdecl f(void); int (__stdcall *p)(int);\n",
    ),
    (
        MsvcFeature::TypeQualifiers,
        "__w64 int a; int * __ptr32 __sptr b; int * __ptr64 __uptr c; __unaligned int *d;\n",
    ),
    (
        MsvcFeature::Inline,
        "__forceinline int f(void) { return 1; }\n",
    ),
    (
        MsvcFeature::Seh,
        "int f(void) { __try { __leave; } __except(1) {} __try {} __finally {} }\n",
    ),
    (
        MsvcFeature::Asm,
        "int f(void) { __asm { mov eax, [ebx + (4)] }\n_asm nop\n}\n",
    ),
    (
        MsvcFeature::AnonymousStructs,
        "struct T { int x; }; struct S { struct T; union U { int y; }; };\n",
    ),
];

#[test]
fn asm_alias_selects_gnu_parentheses_and_msvc_blocks_or_instructions() {
    let source = "int f(void){ __asm(\"nop\"); __asm { nop }\n__asm mov eax, 1\n_asm { nop \
                  }\n_asm mov eax, 2\nreturn 0; }\n";
    for gnu in [false, true] {
        for policy in [
            ExtensionPolicy::Allow,
            ExtensionPolicy::Warn,
            ExtensionPolicy::Deny,
        ] {
            with_parse_configuration(
                source,
                CompilerConfiguration::new(CStandard::C17, policy)
                    .with_gnu_extensions(gnu)
                    .with_msvc_feature(MsvcFeature::Asm, true),
                |p| {
                    assert_eq!(parser_errors(p).count(), 0, "{:?}", p.errors);
                    assert_eq!(
                        p.parser.syntax.iter::<super::super::gnu::Asm<'_>>().count(),
                        1
                    );
                    assert_eq!(p.parser.syntax.iter::<MsAsm<'_>>().count(), 4);
                    let extensions: Vec<_> = p
                        .errors
                        .iter()
                        .filter_map(|x| match x {
                            | crate::translation_phases::TranslationError::Extension(x) =>
                                Some(x.to_string()),
                            | _ => None,
                        })
                        .collect();
                    if policy == ExtensionPolicy::Allow {
                        assert_eq!(extensions.len(), 0, "{extensions:?}");
                    } else {
                        assert_eq!(
                            extensions
                                .iter()
                                .filter(|x| x.contains("GNU extension"))
                                .count(),
                            1
                        );
                        assert_eq!(
                            extensions
                                .iter()
                                .filter(|x| x.contains("MSVC extension"))
                                .count(),
                            4
                        );
                    }
                },
            );
        }
    }
}

#[test]
fn each_parser_feature_requires_only_its_own_flag() {
    for &(feature, source) in FEATURES {
        for standard in [
            CStandard::C89,
            CStandard::C99,
            CStandard::C17,
            CStandard::C23,
            CStandard::C2y,
        ] {
            for gnu in [false, true] {
                let config = CompilerConfiguration::new(standard, ExtensionPolicy::Allow)
                    .with_gnu_extensions(gnu);
                with_parse_configuration(source, config.with_msvc_feature(feature, true), |p| {
                    assert_eq!(
                        parser_errors(p).count(),
                        0,
                        "{feature:?} {standard:?}: {:?}",
                        p.errors
                    );
                });
                with_parse_configuration(
                    source,
                    config
                        .with_msvc_extensions(true)
                        .with_msvc_feature(feature, false),
                    |p| {
                        assert!(
                            parser_errors(p).count() > 0,
                            "disabled {feature:?} accepted {source}"
                        );
                    },
                );
            }
        }
    }
}

#[test]
fn enabled_features_keep_the_ast_under_all_extension_policies() {
    for &(feature, source) in FEATURES {
        for policy in [
            ExtensionPolicy::Allow,
            ExtensionPolicy::Warn,
            ExtensionPolicy::Deny,
        ] {
            with_parse_configuration(
                source,
                CompilerConfiguration::new(CStandard::C17, policy).with_msvc_feature(feature, true),
                |p| {
                    assert_eq!(parser_errors(p).count(), 0, "{feature:?}: {:?}", p.errors);
                    let extensions: Vec<_> = p
                        .errors
                        .iter()
                        .filter_map(|x| {
                            if let crate::translation_phases::TranslationError::Extension(x) = x {
                                Some(x)
                            } else {
                                None
                            }
                        })
                        .collect();
                    if policy == ExtensionPolicy::Allow {
                        assert_eq!(extensions.len(), 0, "{extensions:?}");
                    } else {
                        assert!(!extensions.is_empty(), "{feature:?}");
                        assert!(
                            extensions
                                .iter()
                                .all(|x| x.to_string().contains("MSVC extension")),
                            "{extensions:?}"
                        );
                        assert!(extensions.iter().all(|x| x.severity()
                            == if policy == ExtensionPolicy::Warn {
                                crate::translation_phases::ErrorSeverity::Warning
                            } else {
                                crate::translation_phases::ErrorSeverity::Error
                            }));
                    }
                },
            );
        }
    }
}

#[test]
fn disabled_keyword_spellings_remain_identifiers_and_gnu_inline_is_independent() {
    let source = "int __declspec, __int8, __int16, __int32, __int64, __cdecl, __stdcall, \
                  __fastcall, __vectorcall, __thiscall, __ptr32, __ptr64, __unaligned, __w64, \
                  __sptr, __uptr, __forceinline, __try, __except, __finally, __leave, _asm;\n";
    with_parse_configuration(source, CompilerConfiguration::default(), |p| {
        assert!(p.errors.is_empty(), "{:?}", p.errors);
    });
    for enabled in [false, true] {
        with_parse_configuration(
            "__inline int f(void) { return 1; }\n",
            CompilerConfiguration::default().with_msvc_feature(MsvcFeature::Inline, enabled),
            |p| {
                assert!(p.errors.is_empty(), "{:?}", p.errors);
                assert!(
                    super::function_definition(p, 0)
                        .declaration_specifiers
                        .function_specifiers
                        .is_inline
                );
            },
        );
    }
}

#[test]
fn declspec_shares_attributes_and_preserves_balanced_arguments() {
    let source = "__declspec(dllexport align(16)) int x; struct __declspec(align(8)) S { \
                  __declspec(deprecated(\"reason\")) int x; }; int (__declspec(noinline) \
                  *p)(void);\n";
    with_parse_configuration(
        source,
        CompilerConfiguration::default().with_msvc_feature(MsvcFeature::Declspec, true),
        |p| {
            assert!(p.errors.is_empty(), "{:?}", p.errors);
            let attributes: Vec<_> = p.parser.syntax.iter::<AttributeSpecifier<'_>>().collect();
            assert_eq!(attributes.len(), 4);
            assert!(
                attributes
                    .iter()
                    .all(|x| x.syntax == AttributeSyntax::Msvc && !x.recovered)
            );
            assert_eq!(
                super::sourced_text(p, attributes[0].source_vectors),
                "__declspec(dllexportalign(16))"
            );
            assert_eq!(attributes[0].tokens.len(), 8);
        },
    );
}

#[test]
fn fixed_width_types_preserve_width_and_order_independent_signedness() {
    for (word, width) in [
        ("__int8", 8),
        ("__int16", 16),
        ("__int32", 32),
        ("__int64", 64),
    ] {
        for (prefix, suffix, signedness) in [
            ("", "", None),
            ("signed ", "", Some(true)),
            ("unsigned ", "", Some(false)),
            ("", " signed", Some(true)),
            ("", " unsigned", Some(false)),
        ] {
            with_parse_configuration(
                &format!("{prefix}{word}{suffix} x;\n"),
                CompilerConfiguration::default().with_msvc_feature(MsvcFeature::IntTypes, true),
                |p| {
                    assert!(p.errors.is_empty(), "{:?}", p.errors);
                    assert!(
                        matches!(super::declaration(p,0).declaration_specifiers.type_specifiers, TypeSpecifiers::Extended(ExtendedType::MsInteger { width: w, signedness: s }) if *w == width && *s == signedness)
                    );
                },
            );
        }
    }
}

#[test]
fn calling_conventions_cover_functions_pointers_parameters_and_abstract_declarators() {
    for convention in [
        "__cdecl",
        "__stdcall",
        "__fastcall",
        "__vectorcall",
        "__thiscall",
    ] {
        let source = format!(
            "int {convention} f(void) {{ return 1; }} int ({convention} *p)(int); int \
             (*{convention} q)(int); int g(int ({convention} *cb)(int)) {{ return sizeof(int \
             ({convention} *)(int)); }}\n"
        );
        with_parse_configuration(
            &source,
            CompilerConfiguration::default()
                .with_msvc_feature(MsvcFeature::CallingConventions, true),
            |p| {
                assert!(p.errors.is_empty(), "{source}: {:?}", p.errors);
                assert!(
                    p.parser
                        .syntax
                        .iter::<DirectDeclarator<'_>>()
                        .any(|x| matches!(x, DirectDeclarator::MsModifier(..)))
                );
            },
        );
    }
}

#[test]
fn pointer_modifiers_belong_to_their_pointer_level() {
    with_parse_configuration(
        "int * __ptr32 __sptr * __ptr64 __uptr p; __unaligned __w64 int q;\n",
        CompilerConfiguration::default().with_msvc_feature(MsvcFeature::TypeQualifiers, true),
        |p| {
            assert!(p.errors.is_empty(), "{:?}", p.errors);
            let pointer = super::declaration(p, 0).init_declarators[0]
                .declarator
                .pointer;
            assert_eq!(
                pointer
                    .levels
                    .iter()
                    .map(|x| x.qualifiers)
                    .collect::<Vec<_>>(),
                [
                    TypeQualifiers::PTR32 | TypeQualifiers::SPTR,
                    TypeQualifiers::PTR64 | TypeQualifiers::UPTR
                ]
            );
            assert_eq!(
                super::declaration(p, 1)
                    .declaration_specifiers
                    .type_qualifiers,
                TypeQualifiers::UNALIGNED | TypeQualifiers::W64
            );
        },
    );
}

#[test]
fn seh_nodes_preserve_filter_handler_and_nested_scope() {
    let source = "typedef int T; int f(void) { __try { int T; __try { __leave; } __finally {} } \
                  __except((1,2)) { T x; } T y; return 0; }\n";
    with_parse_configuration(
        source,
        CompilerConfiguration::default().with_msvc_feature(MsvcFeature::Seh, true),
        |p| {
            assert!(p.errors.is_empty(), "{:?}", p.errors);
            let nodes: Vec<_> = p.parser.syntax.iter::<Seh<'_>>().collect();
            assert_eq!(nodes.len(), 2);
            assert!(nodes[0].filter.is_none());
            assert!(nodes[1].filter.is_some());
            assert!(
                nodes.iter().all(|x| !x.body.recovered
                    && !x.handler.recovered
                    && x.handler_keyword.is_some())
            );
            assert!(
                p.parser
                    .syntax
                    .iter::<Statement<'_>>()
                    .any(|x| x.kind == StatementType::SehLeave)
            );
        },
    );
}

#[test]
fn asm_captures_balanced_blocks_lines_and_keyword_separators() {
    let source = "int f(void) { __asm { mov eax, [ebx+(4)] { nop } }\n__asm mov eax, 1 __asm add \
                  eax, 2\n_asm nop\nreturn 0; }\n";
    with_parse_configuration(
        source,
        CompilerConfiguration::default().with_msvc_feature(MsvcFeature::Asm, true),
        |p| {
            assert!(p.errors.is_empty(), "{:?}", p.errors);
            let nodes: Vec<_> = p.parser.syntax.iter::<MsAsm<'_>>().collect();
            assert_eq!(nodes.len(), 4);
            assert!(nodes[0].braced);
            assert!(nodes[1..].iter().all(|x| !x.braced && !x.recovered));
            assert_eq!(
                super::sourced_text(p, nodes[1].source_vectors),
                "__asmmoveax,1"
            );
            assert_eq!(
                super::sourced_text(p, nodes[2].source_vectors),
                "__asmaddeax,2"
            );
            assert!(
                matches!(super::function_definition(p,0).body.kind, StatementType::Compound { items } if items.len() == 5)
            );
        },
    );
}

#[test]
fn malformed_vendor_constructs_preserve_following_input() {
    for source in [
        "__declspec(align(8) int bad; int following;\n",
        "__declspec int bad; int following;\n",
        "unsigned signed __int8 bad; int following;\n",
        "int f(void) { __try {} return 0; } int following;\n",
        "int f(void) { __try __finally {} return 0; } int following;\n",
        "int f(void) { __try {} __except() {} return 0; } int following;\n",
        "int f(void) { __try {} __except(1) return 0; } int following;\n",
        "int f(void) { __leave return 0; } int following;\n",
        "int f(void) { __asm mov eax, [ebx\nreturn 0; } int following;\n",
        "int f(void) { __asm { mov eax, [ebx } return 0; } int following;\n",
    ] {
        with_parse_configuration(
            source,
            CompilerConfiguration::default().with_msvc_extensions(true),
            |p| {
                assert!(parser_errors(p).count() > 0, "{source}");
                assert!(p.items.iter().any(|x| matches!(x, super::super::ExternalDeclaration::Declaration(d) if d.init_declarators.iter().any(|i| i.declarator.identifier().is_some_and(|name| p.parser.context.string_cache.at(name.name) == "following")))), "lost following: {source}: {:?}", p.errors);
            },
        );
    }
}

#[test]
fn every_vendor_input_prefix_terminates_and_restores_machine_state() {
    for &(_, source) in FEATURES {
        for end in 0..source.len() {
            let prefix = format!("{}\n", &source[..end]);
            with_parse_configuration(
                &prefix,
                CompilerConfiguration::default().with_msvc_extensions(true),
                |p| {
                    assert!(p.parser.frames.is_empty());
                    assert!(p.parser.returned.is_none());
                    assert_eq!(p.parser.scopes.depth(), 0);
                },
            );
        }
    }
}

#[test]
fn deeply_nested_seh_uses_the_explicit_frame_stack() {
    let source = format!(
        "int f(void){{ {} __leave; {} }}\n",
        "__try {".repeat(256),
        "} __finally {}".repeat(256)
    );
    with_parse_configuration(
        &source,
        CompilerConfiguration::default().with_msvc_feature(MsvcFeature::Seh, true),
        |p| assert!(p.errors.is_empty(), "{:?}", p.errors),
    );
}

#[test]
fn asm_line_boundaries_use_macro_invocations_and_ignore_splices() {
    for source in [
        "#define INSTR mov eax, 1\nint f(void){ __asm INSTR\nreturn 0; }\n",
        "#define ASM __asm mov eax, 1\nint f(void){ ASM\nreturn 0; }\n",
        "int f(void){ __asm mov eax, \\\n1\nreturn 0; }\n",
        "int f(void){ __asm mov eax, ??/\n1\nreturn 0; }\n",
    ] {
        with_parse_configuration(
            source,
            CompilerConfiguration::default().with_msvc_feature(MsvcFeature::Asm, true),
            |p| {
                assert!(p.errors.is_empty(), "{source}: {:?}", p.errors);
                let nodes: Vec<_> = p.parser.syntax.iter::<MsAsm<'_>>().collect();
                assert_eq!(nodes.len(), 1);
                assert_eq!(nodes[0].tokens.len(), 5);
                assert!(
                    matches!(super::function_definition(p,0).body.kind, StatementType::Compound { items } if items.len() == 2)
                );
            },
        );
    }
}

#[test]
fn braced_asm_keeps_its_closing_brace_after_an_inner_mismatch() {
    for source in [
        "int f(void) { __asm { mov eax, [ebx } return 0; } int g;\n",
        "int f(void) { __asm { mov eax, ([ebx) } return 0; } int g;\n",
        "int f(void) { __asm { mov eax, ebx) } return 0; } int g;\n",
    ] {
        with_parse_configuration(
            source,
            CompilerConfiguration::default().with_msvc_feature(MsvcFeature::Asm, true),
            |p| {
                assert_eq!(parser_errors(p).count(), 1, "{source}: {:?}", p.errors);
                let nodes: Vec<_> = p.parser.syntax.iter::<MsAsm<'_>>().collect();
                assert_eq!(nodes.len(), 1, "{source}");
                assert!(nodes[0].braced && nodes[0].recovered, "{source}");
                assert!(
                    super::sourced_text(p, nodes[0].source_vectors).ends_with('}'),
                    "{source}"
                );
                assert!(
                    matches!(super::function_definition(p, 0).body.kind, StatementType::Compound { items } if items.len() == 2),
                    "{source}"
                );
                assert_eq!(p.items.len(), 2, "{source}");
                assert!(!super::declaration(p, 1).recovered, "{source}");
            },
        );
    }
}

#[test]
fn masm_numbers_in_asm_do_not_raise_c_constant_diagnostics() {
    let asm = CompilerConfiguration::new(CStandard::C17, ExtensionPolicy::Allow)
        .with_msvc_feature(MsvcFeature::Asm, true);
    for (source, expected) in [
        (
            "int f(void){ __asm mov eax, 0FFh\n__asm { mov al, 10h } return 0; }\n",
            0,
        ),
        (
            "int f(void){ __asm { mov eax, 08h\n mov ebx, 1e\n mov ecx, 99999999999999999999 } \
             return 0; }\n",
            0,
        ),
        (
            "#define HEX 0FFh\nint a = 1; int g(void){ return 0; }\nint f(void){ __asm mov eax, \
             HEX\n return 0; }\n",
            0,
        ),
        // The same spellings outside the assembly stay C constants.
        (
            "int x = 0FFh; int f(void){ __asm mov eax, 0FFh\n return 0; }\n",
            1,
        ),
        (
            "#define HEX 0FFh\nint f(void){ __asm mov eax, HEX\n return 0; } int y = HEX;\n",
            1,
        ),
        (
            "#define BOTH(x) int y = x; int f(void){ __asm { mov eax, x } return 0; \
             }\nBOTH(0FFh)\n",
            1,
        ),
    ] {
        with_parse_configuration(source, asm, |p| {
            assert_eq!(p.errors.len(), expected, "{source}: {:?}", p.errors);
            assert_eq!(
                p.parser.syntax.iter::<MsAsm<'_>>().count(),
                source.matches("__asm").count()
            );
        });
    }
    // Pedantic diagnostics about C constant syntax do not apply either.
    with_parse_configuration(
        "int f(void){ __asm mov eax, 0bh\n__asm mov ebx, 1LL\n return 0; }\n",
        CompilerConfiguration::new(CStandard::C89, ExtensionPolicy::Warn)
            .with_gnu_extensions(true)
            .with_msvc_feature(MsvcFeature::Asm, true),
        |p| {
            assert!(
                p.errors.iter().all(|x| x.to_string().contains("'__asm'")),
                "{:?}",
                p.errors
            );
        },
    );
}
