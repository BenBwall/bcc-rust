//! Translation limits, resource limits, and linear source storage.

use std::{
    fmt::Write as _,
    path::PathBuf,
};

use super::{
    block_items,
    declaration,
    function_definition,
    parser_errors,
    with_parse,
    with_parse_limits,
    with_parsed,
};
use crate::{
    translation_phases::{
        Context,
        TranslationError,
        parsing::{
            Parser,
            ParserLimits,
            declaration_syntax::{
                Declaration,
                Declarator,
                DirectDeclarator,
                EnumSpecifier,
                InitDeclarator,
                StructOrUnionSpecifier,
                TypeSpecifiers,
            },
            errors::{
                ParserError,
                ParserErrorType,
                ParserResource,
            },
            scope::{
                NameClass,
                ScopeKind,
                ScopeStack,
            },
            syntax::{
                ExternalDeclaration,
                Statement,
                StatementType,
            },
        },
        preprocessing::Preprocessor,
    },
    util::shared::SharedVec,
};

#[test]
fn an_early_prototype_record_survives_later_parameter_lists() {
    let arena = crate::util::bump::Bump::new();
    let mut scopes = ScopeStack::new_in(&arena);
    for index in 0..100 {
        scopes.enter_scope(ScopeKind::FunctionPrototype);
        scopes.publish((index + 1).into(), NameClass::Typedef);
        scopes.retain_innermost_bindings((index as usize, 1));
        scopes.restore_depth(0);
    }
    scopes.enter_scope(ScopeKind::Function);
    assert!(scopes.publish_retained_bindings((0, 1)));
    assert!(scopes.is_typedef(1.into()));
    assert!(!scopes.is_typedef(100.into()));
}

#[test]
fn parenthesized_declarators_meet_the_c99_floor_and_stress_the_heap_stack() {
    for depth in [63, 4_096] {
        let source = format!("int {}deep{};\n", "(".repeat(depth), ")".repeat(depth));
        with_parse(&source, |parsed| {
            assert_eq!(parsed.items.len(), 1);
            assert!(
                parser_errors(parsed).next().is_none(),
                "{:#?}",
                parsed.errors
            );
            assert!(
                parsed
                    .parser
                    .trace
                    .iter()
                    .map(|event| event.depth)
                    .max()
                    .expect("nonempty trace")
                    > depth,
                "grammar depth must be represented by heap-backed frames"
            );
            assert!(
                parsed.parser.context.source_segment_count() <= depth * 8 + 32,
                "source provenance must grow linearly with grammar depth"
            );
        });
    }
}

#[test]
fn long_statement_lists_keep_source_storage_linear() {
    let count = 1_024;
    with_parse(
        &format!("void f(void) {{ {} }}\n", "0; ".repeat(count)),
        |parsed| {
            assert_eq!(
                block_items(function_definition(parsed, 0).body).len(),
                count
            );
            assert!(
                parser_errors(parsed).next().is_none(),
                "{:#?}",
                parsed.errors
            );
            assert!(
                parsed.parser.context.source_segment_count() < count * 64,
                "block provenance copied each growing prefix: {} segments",
                parsed.parser.context.source_segment_count()
            );
        },
    );
}

#[test]
fn source_storage_exhaustion_reports_one_resource_diagnostic() {
    with_parse_limits(
        // Enough tokens to pass the limit with the preprocessor arena
        // compacted between tokens.
        "int a; int b; int c; int d; int e; int f;\n",
        ParserLimits {
            source_segments: 10,
            ..ParserLimits::default()
        },
        |parsed| {
            assert_eq!(
                parser_errors(parsed)
                    .filter(|error| matches!(
                        error,
                        ParserErrorType::ResourceLimitExceeded {
                            resource: ParserResource::SourceSegments,
                            limit:    10,
                        }
                    ))
                    .count(),
                1
            );
            assert!(
                parsed.parser.frames.is_empty(),
                "resource exit retained frames"
            );
            assert_eq!(parsed.parser.scopes.depth(), 0);
        },
    );
}

#[test]
fn source_storage_exhaustion_stops_preprocessing_the_remaining_input() {
    let tu = crate::util::bump::Bump::new();
    let mut context = Context::new(&tu);
    let preprocess_arena = crate::util::bump::Bump::new();
    let parse_arena = crate::util::bump::Bump::new();
    let preprocessor = Preprocessor::new(
        &preprocess_arena,
        &mut context,
        PathBuf::from("<limit-test>").into_boxed_path(),
        &"int a;\n".repeat(1_000),
        SharedVec::default(),
        SharedVec::default(),
    );
    let mut parser = Parser::new_with_limits(
        preprocessor,
        &mut context,
        ParserLimits {
            source_segments: 10,
            ..ParserLimits::default()
        },
        &parse_arena,
    );
    let mut items = 0;
    while parser.next_item().is_some() {
        items += 1;
    }
    assert!(items < 2, "{items} declarations parsed after the limit");
    let errors = std::iter::from_fn(|| context.pop_pending_error()).collect::<Vec<_>>();
    assert_eq!(
        errors
            .iter()
            .filter(|error| matches!(
                error,
                TranslationError::Parsing(ParserError {
                    error_type: ParserErrorType::ResourceLimitExceeded {
                        resource: ParserResource::SourceSegments,
                        limit:    10,
                    },
                    ..
                })
            ))
            .count(),
        1
    );
    assert!(
        context.source_segment_count() < 100,
        "preprocessing kept growing provenance after the limit: {} segments",
        context.source_segment_count()
    );
}

#[test]
fn macro_expansion_stops_at_the_source_segment_limit() {
    let mut source = String::from("#define X0 x\n");
    for level in 1..=12 {
        writeln!(source, "#define X{level} X{} X{}", level - 1, level - 1).unwrap();
    }
    source.push_str("X12\n");

    with_parse_limits(
        &source,
        ParserLimits {
            source_segments: 30,
            ..ParserLimits::default()
        },
        |parsed| {
            assert!(
                parsed.parser.context.source_segment_count() < 256,
                "{} source segments retained",
                parsed.parser.context.source_segment_count()
            );
            assert_eq!(
                parser_errors(parsed)
                    .filter(|error| matches!(
                        error,
                        ParserErrorType::ResourceLimitExceeded {
                            resource: ParserResource::SourceSegments,
                            limit:    30,
                        }
                    ))
                    .count(),
                1
            );
            assert!(matches!(
                parsed.items.as_slice(),
                [ExternalDeclaration::Error(_)]
            ));
        },
    );
}

#[test]
fn adjacent_strings_stop_at_the_source_segment_limit() {
    let mut source = String::from("#define X0 \"x\"\n");
    for level in 1..=12 {
        writeln!(source, "#define X{level} X{} X{}", level - 1, level - 1).unwrap();
    }
    source.push_str("const char *value = X12;\n");

    with_parse_limits(
        &source,
        ParserLimits {
            source_segments: 1_000,
            ..ParserLimits::default()
        },
        |parsed| {
            assert!(
                parsed.parser.context.source_segment_count() < 1_200,
                "{} source segments retained",
                parsed.parser.context.source_segment_count()
            );
            assert!(parser_errors(parsed).any(|error| matches!(
                error,
                ParserErrorType::ResourceLimitExceeded {
                    resource: ParserResource::SourceSegments,
                    limit:    1_000,
                }
            )));
        },
    );
}

#[test]
fn long_declaration_lists_keep_source_storage_linear() {
    let count = 1_024;
    let names = (0..count).map(|i| format!("p{i}")).collect::<Vec<_>>();
    for source in [
        format!("int {};\n", names.join(",")),
        format!("void f(void) {{ g({}); }}\n", vec!["0"; count].join(",")),
        format!("int values[] = {{ {} }};\n", vec!["0"; count].join(",")),
        format!("const char *text = {};\n", "\"a\" ".repeat(count)),
        format!(
            "void f({});\n",
            names
                .iter()
                .map(|name| format!("int {name}"))
                .collect::<Vec<_>>()
                .join(",")
        ),
        format!("struct S {{ int {}; }};\n", names.join("; int ")),
        format!("enum E {{ {} }};\n", names.join(",")),
    ] {
        with_parse(&source, |parsed| {
            assert!(
                parser_errors(parsed).next().is_none(),
                "{:#?}",
                parsed.errors
            );
            assert!(
                parsed.parser.context.source_segment_count() < count * 64,
                "list provenance copied each growing prefix: {} segments",
                parsed.parser.context.source_segment_count()
            );
        });
    }
}

#[test]
fn source_storage_is_linear_in_token_count() {
    // Frames merge provenance one token or child at a time; each fetched
    // token must be stored once rather than re-copied for every enclosing
    // construct.
    let source_segments = |count: usize| {
        let source = format!(
            "struct S {{ int a : 3; int (*f)(int); }};\nint f(int x) {{ int t[2] = {{ [1] = {}x{} \
             }}; {} return t[0]; }}\n",
            "(".repeat(count),
            ")".repeat(count),
            "if (x) { x = (x + 1) * (x - 2) / 3; } ".repeat(count),
        );
        with_parse(&source, |parsed| {
            assert!(
                parser_errors(parsed).next().is_none(),
                "{:#?}",
                parsed.errors
            );
            parsed.parser.context.source_segment_count()
        })
    };
    let small = source_segments(256);
    let large = source_segments(1_024);
    assert!(
        large <= small * 5,
        "source storage grew superlinearly: {small} -> {large} segments"
    );
}

#[test]
fn configured_external_declaration_limit_has_boundary_evidence() {
    let limits = ParserLimits {
        external_declarations: 2,
        ..ParserLimits::default()
    };
    for source in ["int a;\n", "int a; int b;\n"] {
        with_parse_limits(source, limits, |parsed| {
            assert!(
                parser_errors(parsed)
                    .all(|error| !matches!(error, ParserErrorType::ResourceLimitExceeded { .. }))
            );
        });
    }

    with_parse_limits("int a; int b; int c;\n", limits, |parsed| {
        assert!(matches!(
            parsed.items.last(),
            Some(ExternalDeclaration::Error(_))
        ));
        assert!(parser_errors(parsed).any(|error| matches!(
            error,
            ParserErrorType::ResourceLimitExceeded {
                resource: ParserResource::ExternalDeclarations,
                limit:    2,
            }
        )));
        assert!(parsed.parser.frames.is_empty());
        assert!(parsed.parser.returned.is_none());
        assert_eq!(parsed.parser.scopes.depth(), 0);
    });
}

#[test]
fn configured_node_and_frame_limits_fail_with_stable_diagnostics() {
    with_parse_limits(
        "int value = 1;\n",
        ParserLimits {
            syntax_nodes: 0,
            ..ParserLimits::default()
        },
        |parsed| {
            assert!(parser_errors(parsed).any(|error| matches!(
                error,
                ParserErrorType::ResourceLimitExceeded {
                    resource: ParserResource::SyntaxNodes,
                    limit:    0,
                }
            )));
            assert!(matches!(
                parsed.items.last(),
                Some(ExternalDeclaration::Error(_))
            ));
            assert_resource_limit_cleanup(parsed);
        },
    );

    with_parse_limits(
        "int value;\n",
        ParserLimits {
            frame_depth: 1,
            ..ParserLimits::default()
        },
        |parsed| {
            assert!(parser_errors(parsed).any(|error| matches!(
                error,
                ParserErrorType::ResourceLimitExceeded {
                    resource: ParserResource::FrameDepth,
                    limit:    1,
                }
            )));
            assert!(matches!(
                parsed.items.last(),
                Some(ExternalDeclaration::Error(_))
            ));
            assert_resource_limit_cleanup(parsed);
        },
    );

    let defaults = ParserLimits::default();
    assert_eq!(defaults.external_declarations, usize::MAX);
    assert_eq!(defaults.syntax_nodes, usize::MAX);
    assert_eq!(defaults.frame_depth, u32::MAX as usize);
    assert_eq!(defaults.source_segments, usize::MAX);
}

fn assert_resource_limit_cleanup(parsed: &super::Parsed<'_, '_>) {
    assert!(parsed.parser.frames.is_empty());
    assert!(parsed.parser.returned.is_none());
    assert!(parsed.parser.recovery.active.is_none());
    assert_eq!(parsed.parser.scopes.depth(), 0);
    assert_eq!(
        parser_errors(parsed)
            .filter(|error| matches!(error, ParserErrorType::ResourceLimitExceeded { .. }))
            .count(),
        1
    );
    assert!(
        parser_errors(parsed).all(|error| !matches!(error, ParserErrorType::EmptyTranslationUnit))
    );
}

#[test]
fn mixed_declarator_translation_floor_uses_the_typed_tree() {
    fn count_derivations(declarator: Declarator<'_>) -> (usize, usize, usize) {
        let mut counts = (declarator.pointer.type_qualifiers_list.len(), 0, 0);
        for direct in declarator.kind {
            match *direct {
                | DirectDeclarator::Parenthesized(parenthesized) => {
                    let nested = count_derivations(parenthesized.declarator);
                    counts.0 += nested.0;
                    counts.1 += nested.1;
                    counts.2 += nested.2;
                },
                | DirectDeclarator::Array { .. } => counts.1 += 1,
                | DirectDeclarator::Function { .. }
                | DirectDeclarator::KAndRStyleFunction { .. } => counts.2 += 1,
                | DirectDeclarator::Identifier(_) => {},
            }
        }
        counts
    }

    with_parsed(
        "struct Incomplete (*(*(*(*(*(*value)[1])(void))(void))(void))(void))(void);\n",
        |unit, context| {
            assert!(context.take_pending_errors().is_empty());
            let ExternalDeclaration::Declaration(root) = unit.external_declarations()[0] else {
                panic!("expected a declaration root")
            };
            let TypeSpecifiers::StructOrUnion(specifier) =
                root.declaration_specifiers.type_specifiers
            else {
                panic!("expected an incomplete structure base type")
            };
            assert!(specifier.struct_declaration_list.is_none());
            let declarator = root.init_declarators[0].declarator;

            assert_eq!(count_derivations(declarator), (6, 1, 5));
        },
    );
}

fn assert_syntax_node_limit_boundary(source: &str) {
    let exact_limit = with_parse(source, |baseline| {
        assert!(
            parser_errors(baseline).next().is_none(),
            "baseline failed for {source:?}: {:?}",
            baseline.errors
        );
        baseline.parser.syntax.node_count()
    });
    assert!(exact_limit > 0, "fixture must retain syntax nodes");

    with_parse_limits(
        source,
        ParserLimits {
            syntax_nodes: exact_limit,
            ..ParserLimits::default()
        },
        |exact| {
            assert!(parser_errors(exact).all(|error| !matches!(
                error,
                ParserErrorType::ResourceLimitExceeded {
                    resource: ParserResource::SyntaxNodes,
                    ..
                }
            )));
            assert_eq!(exact.parser.syntax.node_count(), exact_limit);
        },
    );

    let below_limit = exact_limit - 1;
    with_parse_limits(
        source,
        ParserLimits {
            syntax_nodes: below_limit,
            ..ParserLimits::default()
        },
        |below| {
            assert!(parser_errors(below).any(|error| matches!(
                error,
                ParserErrorType::ResourceLimitExceeded {
                    resource: ParserResource::SyntaxNodes,
                    limit,
                } if *limit == below_limit
            )));
            // The step that crossed the limit keeps its nodes (the tree
            // arena cannot roll back), but parsing stops there: the error
            // root is the last item, and the parser never allocates more
            // than the complete parse needs.
            assert!(
                below.parser.syntax.node_count() <= exact_limit,
                "parsing continued past the syntax limit for {source:?}: limit={below_limit}, \
                 complete parse={exact_limit}, actual={}",
                below.parser.syntax.node_count()
            );
            assert!(matches!(
                below.items.last(),
                Some(ExternalDeclaration::Error(_))
            ));
        },
    );
}

#[test]
fn every_frame_retained_list_respects_the_exact_syntax_node_limit() {
    for source in [
        "int *const value[2];\n",
        "int function(int first, int second);\n",
        "struct S { int first, second; };\n",
        "enum E { A, B };\n",
        "int f(void) { g(1, 2); }\n",
        "int values[2] = { [0] = 1, [1] = 2 };\n",
        "int old(first, second) int first; int second; { return first; }\n",
        "int block(void) { int value; value = 1; return value; }\n",
    ] {
        assert_syntax_node_limit_boundary(source);
    }
}

#[test]
fn configured_frame_depth_accepts_exact_limit_and_rejects_limit_plus_one() {
    let source = "int f(void) { return sizeof(int (*)[2]) + ((1 + 2) * 3); }\n";
    let exact_limit = with_parse(source, |baseline| {
        baseline
            .parser
            .trace
            .iter()
            .map(|event| event.depth)
            .max()
            .expect("fixture must execute parser frames")
    });

    with_parse_limits(
        source,
        ParserLimits {
            frame_depth: exact_limit,
            ..ParserLimits::default()
        },
        |exact| {
            assert!(parser_errors(exact).all(|error| !matches!(
                error,
                ParserErrorType::ResourceLimitExceeded {
                    resource: ParserResource::FrameDepth,
                    ..
                }
            )));
        },
    );

    with_parse_limits(
        source,
        ParserLimits {
            frame_depth: exact_limit - 1,
            ..ParserLimits::default()
        },
        |below| {
            assert!(parser_errors(below).any(|error| matches!(
                error,
                ParserErrorType::ResourceLimitExceeded {
                    resource: ParserResource::FrameDepth,
                    limit,
                } if *limit == exact_limit - 1
            )));
        },
    );
}

#[test]
fn syntax_limit_counts_nodes_retained_by_active_frames() {
    let limit = 2;
    with_parse_limits(
        "enum E { A, B, C };\n",
        ParserLimits {
            syntax_nodes: limit,
            ..ParserLimits::default()
        },
        |parsed| {
            assert!(parser_errors(parsed).any(|error| matches!(
                error,
                ParserErrorType::ResourceLimitExceeded {
                    resource: ParserResource::SyntaxNodes,
                    limit: actual,
                } if *actual == limit
            )));
            assert!(parsed.parser.syntax.node_count() <= limit);
            assert!(matches!(
                parsed.items.as_slice(),
                [ExternalDeclaration::Error(_)]
            ));
        },
    );
}

#[test]
fn remaining_c99_parser_translation_floors_are_supported() {
    let derived = format!("int value{};\n", "[1]".repeat(12));
    with_parse(&derived, |derived| {
        assert!(parser_errors(derived).next().is_none());
        let derived_declarator = &derived
            .parser
            .syntax
            .nth::<InitDeclarator<'_>>(0)
            .declarator;
        assert_eq!(
            derived_declarator
                .kind
                .iter()
                .filter(|direct| matches!(direct, DirectDeclarator::Array { .. }))
                .count(),
            12
        );
    });

    let block_identifiers = (0..511)
        .map(|index| format!("b{index}"))
        .collect::<Vec<_>>()
        .join(",");
    let block = format!("int f(void) {{ int {block_identifiers}; return 0; }}\n");
    with_parse(&block, |block| {
        assert!(parser_errors(block).next().is_none());
        let block_declaration = block
            .parser
            .syntax
            .iter::<Declaration<'_>>()
            .max_by_key(|declaration| declaration.init_declarators.len())
            .expect("block fixture must contain declarations");
        assert_eq!(block_declaration.init_declarators.len(), 511);
        let last_block_init = block_declaration
            .init_declarators
            .last()
            .expect("nonempty syntax list");
        assert_eq!(
            last_block_init
                .declarator
                .identifier()
                .map(|identifier| block.parser.context.string_cache.at(identifier.name)),
            Some("b510")
        );
    });

    let external_identifiers = (0..4095)
        .map(|index| format!("e{index}"))
        .collect::<Vec<_>>()
        .join(",");
    let external = format!("int {external_identifiers};\n");
    with_parse(&external, |external| {
        assert!(parser_errors(external).next().is_none());
        let declaration = declaration(external, 0);
        assert_eq!(declaration.init_declarators.len(), 4_095);
        let last_external_init = declaration
            .init_declarators
            .last()
            .expect("nonempty syntax list");
        assert_eq!(
            last_external_init
                .declarator
                .identifier()
                .map(|identifier| external.parser.context.string_cache.at(identifier.name)),
            Some("e4094")
        );
    });

    let cases = (0..1023).fold(String::new(), |mut cases, index| {
        write!(cases, "case {index}: ;").expect("writing to a String cannot fail");
        cases
    });
    let switch = format!("int f(int x) {{ switch (x) {{ {cases} }} return 0; }}\n");
    with_parse(&switch, |switch| {
        assert!(parser_errors(switch).next().is_none());
        assert_eq!(
            switch
                .parser
                .syntax
                .iter::<Statement<'_>>()
                .filter(|statement| matches!(statement.kind, StatementType::Case(..)))
                .count(),
            1_023
        );
    });

    let members = (0..1023).fold(String::new(), |mut members, index| {
        write!(members, "int m{index};").expect("writing to a String cannot fail");
        members
    });
    let structure = format!("struct S {{ {members} }};\n");
    with_parse(&structure, |structure| {
        assert!(parser_errors(structure).next().is_none());
        let member_list = structure
            .parser
            .syntax
            .nth::<StructOrUnionSpecifier<'_>>(0)
            .struct_declaration_list
            .expect("struct definition must retain members");
        assert_eq!(member_list.len(), 1_023);
        let last_member = member_list.last().expect("nonempty syntax list");
        let last_member_declarator = last_member.struct_declarator_list[0];
        assert_eq!(
            last_member_declarator
                .declarator
                .and_then(Declarator::identifier)
                .map(|identifier| structure.parser.context.string_cache.at(identifier.name)),
            Some("m1022")
        );
    });

    let enumerators = (0..1023)
        .map(|index| format!("E{index}"))
        .collect::<Vec<_>>()
        .join(",");
    let enumeration = format!("enum E {{ {enumerators} }};\n");
    with_parse(&enumeration, |enumeration| {
        assert!(parser_errors(enumeration).next().is_none());
        let enumeration_list = enumeration
            .parser
            .syntax
            .nth::<EnumSpecifier<'_>>(0)
            .enumeration_list
            .expect("enum definition must retain enumerators");
        assert_eq!(enumeration_list.len(), 1_023);
        let last_enumerator = enumeration_list.last().expect("nonempty syntax list");
        assert_eq!(
            enumeration
                .parser
                .context
                .string_cache
                .at(last_enumerator.name.name),
            "E1022"
        );
    });

    let mut nested = "int leaf;".to_owned();
    for index in (0..63).rev() {
        nested = format!("struct S{index} {{ {nested} }} member{index};");
    }
    let nested = format!("struct Outer {{ {nested} }};\n");
    with_parse(&nested, |nested| {
        assert!(parser_errors(nested).next().is_none());
        assert_eq!(
            nested.parser.syntax.count::<StructOrUnionSpecifier<'_>>(),
            64
        );
        assert_eq!(
            nested
                .parser
                .syntax
                .iter::<StructOrUnionSpecifier<'_>>()
                .last()
                .and_then(|specifier| specifier.identifier)
                .map(|identifier| nested.parser.context.string_cache.at(identifier.name)),
            Some("Outer")
        );
    });
}

#[test]
fn completed_roots_release_macro_hint_metadata() {
    // The whole unit is preprocessed first, so the hints of every invocation
    // exist at once; each completed root releases those before it.
    let source = format!("#define DECL(x) int x\n{}", "DECL(value);\n".repeat(5_000));
    with_parse(&source, |parsed| {
        assert!(parsed.errors.is_empty(), "{:?}", parsed.errors);
        assert_eq!(parsed.items.len(), 5_000);
        assert!(
            parsed.parser.context.macro_hint_entries() < 16,
            "completed roots kept {} macro hints",
            parsed.parser.context.macro_hint_entries()
        );
    });
}
