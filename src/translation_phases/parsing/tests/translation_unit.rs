//! Whole-translation-unit syntax trees, inspection, and truncation.

use std::path::PathBuf;

use proptest::prelude::*;

use super::{
    Parsed,
    parser_errors,
    with_parse,
    with_parsed,
};
use crate::{
    translation_phases::{
        Context,
        GetSourceVectors,
        SourceVector,
        TranslationError,
        TranslationPhase,
        parsing::{
            Parser,
            declaration_syntax::DirectDeclarator,
            errors::ParserErrorType,
            inspection::InspectionOptions,
            syntax::ExternalDeclaration,
        },
        preprocessing::Preprocessor,
    },
    util::shared::SharedVec,
};

#[test]
fn complete_syntax_tree_accepts_parser_issued_empty_lists() {
    let tu = crate::util::bump::Bump::new();
    let mut context = Context::new(&tu);
    let preprocessor = Preprocessor::new(
        &mut context,
        PathBuf::from("<empty-syntax-lists-test>").into_boxed_path(),
        "int f(); int (*pointer)(); int g(void);\n",
        SharedVec::default(),
        SharedVec::default(),
    );

    let unit = Parser::new(preprocessor, &mut context).parse_translation_unit(&mut context);

    assert_eq!(unit.external_declarations().len(), 3);
    assert!(
        context
            .take_pending_errors()
            .iter()
            .all(|error| { !matches!(error, TranslationError::Parsing(_)) })
    );
    let output = unit.syntax().inspect(
        unit.external_declarations(),
        &context,
        InspectionOptions::default(),
    );
    for name in ["f", "pointer", "g"] {
        assert!(output.contains(&format!("declarator {name}")), "{output}");
    }
}

#[test]
fn complete_translation_unit_retains_roots_already_streamed() {
    let tu = crate::util::bump::Bump::new();
    let mut context = Context::new(&tu);
    let preprocessor = Preprocessor::new(
        &mut context,
        PathBuf::from("<mixed-parser-consumption-test>").into_boxed_path(),
        "int first; int second;\n",
        SharedVec::default(),
        SharedVec::default(),
    );
    let mut parser = Parser::new(preprocessor, &mut context);

    assert!(matches!(
        parser.next_item(&mut context),
        Some(ExternalDeclaration::Declaration(_))
    ));
    let unit = parser.parse_translation_unit(&mut context);

    assert_eq!(unit.external_declarations().len(), 2);
    let names = unit
        .external_declarations()
        .iter()
        .map(|root| {
            let ExternalDeclaration::Declaration(index) = *root else {
                panic!("expected a declaration root")
            };
            let declaration = unit.syntax().declaration(index);
            let declarator = declaration.init_declarators()[0].declarator;
            let identifier = unit
                .syntax()
                .direct_declarators(declarator.kind)
                .iter()
                .find_map(|direct| match direct {
                    | DirectDeclarator::Identifier(identifier) => Some(*identifier),
                    | _ => None,
                })
                .expect("named declaration");
            context.string_cache.at(identifier.name)
        })
        .collect::<Vec<_>>();
    assert_eq!(names, ["first", "second"]);
}

#[test]
fn typed_identifier_provenance_survives_macros_and_includes() {
    let tu = crate::util::bump::Bump::new();
    let mut context = Context::new(&tu);
    let include_directory = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("parser");
    let preprocessor = Preprocessor::new(
        &mut context,
        PathBuf::from("<identifier-provenance-test>").into_boxed_path(),
        "#define DECL_NAME generated\nint DECL_NAME;\n#include \"identifier-provenance.h\"\n",
        vec![include_directory.clone()].into(),
        SharedVec::default(),
    );
    let unit = Parser::new(preprocessor, &mut context).parse_translation_unit(&mut context);

    assert!(context.take_pending_errors().is_empty());
    assert_eq!(unit.external_declarations().len(), 2);
    let identifiers = unit
        .external_declarations()
        .iter()
        .map(|root| {
            let ExternalDeclaration::Declaration(index) = *root else {
                panic!("expected a declaration root")
            };
            let declarator = unit.syntax().declaration(index).init_declarators()[0].declarator;
            unit.syntax()
                .direct_declarators(declarator.kind)
                .iter()
                .find_map(|direct| match direct {
                    | DirectDeclarator::Identifier(identifier) => Some(*identifier),
                    | _ => None,
                })
                .expect("named declaration")
        })
        .collect::<Vec<_>>();

    assert_eq!(context.string_cache.at(identifiers[0].name), "generated");
    let macro_vectors = context.get_source_vectors(identifiers[0].source_vectors);
    assert!(
        macro_vectors
            .iter()
            .all(|vector| vector.source_file_index == 0)
    );
    assert_eq!((macro_vectors[0].line, macro_vectors[0].column), (1, 19));

    assert_eq!(
        context.string_cache.at(identifiers[1].name),
        "included_name"
    );
    let include_vectors = context.get_source_vectors(identifiers[1].source_vectors);
    assert_ne!(include_vectors, []);
    assert!(
        include_vectors
            .iter()
            .all(|vector| vector.source_file_index == 1)
    );
    assert_eq!((include_vectors[0].line, include_vectors[0].column), (1, 5));

    let inspected = unit.syntax().inspect(
        unit.external_declarations(),
        &context,
        InspectionOptions {
            show_locations: true,
        },
    );
    assert!(
        inspected.contains("declarator generated @1:19"),
        "{inspected}"
    );
    // Locations outside the main file name their file.
    let header = include_directory.join("identifier-provenance.h");
    assert!(
        inspected.contains(&format!(
            "declarator included_name @{}:1:5",
            header.display()
        )),
        "{inspected}"
    );
}

#[test]
fn deterministic_inspection_uses_spellings_and_marks_recovery() {
    let tu = crate::util::bump::Bump::new();
    let mut context = Context::new(&tu);
    let source = "int good = 1; } int after;\n";
    let preprocessor = Preprocessor::new(
        &mut context,
        PathBuf::from("<inspection-test>").into_boxed_path(),
        source,
        SharedVec::default(),
        SharedVec::default(),
    );
    let unit = Parser::new(preprocessor, &mut context).parse_translation_unit(&mut context);
    let first = unit.syntax().inspect(
        unit.external_declarations(),
        &context,
        InspectionOptions::default(),
    );
    let second = unit.syntax().inspect(
        unit.external_declarations(),
        &context,
        InspectionOptions::default(),
    );

    assert_eq!(first, second);
    let good = first.find("declarator good").expect("good declaration");
    let error = first.find("root[1] error").expect("error root");
    let after = first
        .find("declarator after")
        .expect("following declaration");
    assert!(good < error && error < after, "{first}");
    assert!(!first.contains("StringCacheId"), "{first}");
    assert!(!first.contains("Index("), "{first}");
}

#[test]
fn inspection_traverses_declarators_tags_parameters_and_designations() {
    let tu = crate::util::bump::Bump::new();
    let mut context = Context::new(&tu);
    let source = "struct S { int member : 3; }; int values[2] = { [1] = 7 }; int f(int arg);\n";
    let preprocessor = Preprocessor::new(
        &mut context,
        PathBuf::from("<inspection-shapes-test>").into_boxed_path(),
        source,
        SharedVec::default(),
        SharedVec::default(),
    );
    let unit = Parser::new(preprocessor, &mut context).parse_translation_unit(&mut context);
    let output = unit.syntax().inspect(
        unit.external_declarations(),
        &context,
        InspectionOptions::default(),
    );

    for expected in [
        "struct S",
        "member bit-field",
        "width: constant",
        "array static=false",
        "array-designator",
        "index: constant",
        "function variadic=false",
        "parameter type=int",
        "identifier arg",
    ] {
        assert!(output.contains(expected), "missing {expected:?}:\n{output}");
    }

    let tu = crate::util::bump::Bump::new();
    let mut multi_context = Context::new(&tu);
    let preprocessor = Preprocessor::new(
        &mut multi_context,
        PathBuf::from("<inspection-order-test>").into_boxed_path(),
        "int a = 1, b = 2;\n",
        SharedVec::default(),
        SharedVec::default(),
    );
    let multi =
        Parser::new(preprocessor, &mut multi_context).parse_translation_unit(&mut multi_context);
    let multi = multi.syntax().inspect(
        multi.external_declarations(),
        &multi_context,
        InspectionOptions::default(),
    );
    let a = multi.find("declarator a").expect("first declarator");
    let one = multi.find("constant 1 (int)").expect("first initializer");
    let b = multi.find("declarator b").expect("second declarator");
    let two = multi.find("constant 2 (int)").expect("second initializer");
    assert!(a < one && one < b && b < two, "{multi}");
}

#[test]
fn inspection_has_a_stable_statement_expression_and_missing_slot_golden() {
    let output = with_parsed(
        "int f(void) { if (x) return a + 1; else return 0; if () ; switch (x) { case : ; } }\n",
        |unit, context| {
            unit.syntax().inspect(
                unit.external_declarations(),
                context,
                InspectionOptions::default(),
            )
        },
    );

    let expected = [
        "recovered-function-definition f recovered type=int storage=none qualifiers=none \
         function-specifiers=none",
        "  declarator: declarator pointer-levels=0",
        "    identifier f",
        "    function variadic=false",
        "      parameter type=void storage=none qualifiers=none function-specifiers=none",
        "  body: compound recovered",
        "    block-item: if",
        "      condition: identifier x",
        "      then: return",
        "        return-value: binary +",
        "          lhs: identifier a",
        "          rhs: constant 1 (int)",
        "      else: return",
        "        return-value: constant 0 (int)",
        "    block-item: if recovered",
        "      condition: missing",
        "      then: null",
        "    block-item: switch recovered",
        "      condition: identifier x",
        "      body: compound recovered",
        "        block-item: case recovered",
        "          case-value: missing",
        "          labeled: null",
        "",
    ]
    .join("\n");
    assert_eq!(output, expected);
}

#[test]
fn every_representative_token_truncation_terminates_with_clean_state() {
    let fixtures = [
        "int f(int x) { if (x) return x; else return 0; }",
        "struct S { int x : 3; }; enum E { A = 1, B, };",
        "int a[4] = { [1] = 2, 3 };",
        "typedef int T; int f(void) { return ((T){ 1 }); }",
    ];
    for fixture in fixtures {
        for end in 0..=fixture.len() {
            if !fixture.is_char_boundary(end) {
                continue;
            }
            with_parse(&fixture[..end], |parsed| {
                assert!(parsed.parser.frames.is_empty(), "{fixture:?} at {end}");
                assert!(parsed.parser.returned.is_none(), "{fixture:?} at {end}");
                assert!(
                    parsed.parser.recovery.active.is_none(),
                    "{fixture:?} at {end}"
                );
                assert_eq!(parsed.parser.scopes.depth(), 0, "{fixture:?} at {end}");
                assert!(
                    parsed.parser.label_scopes.is_empty(),
                    "{fixture:?} at {end}"
                );
                assert!(
                    parsed.parser.switch_scopes.is_empty(),
                    "{fixture:?} at {end}"
                );
            });
        }
    }
}

#[test]
fn declaration_only_specifiers_in_struct_members_make_progress() {
    for specifier in ["typedef", "extern", "inline"] {
        let source = format!("struct {{ {specifier} int member; }}; int after;");
        with_parse(&source, |parsed| {
            assert!(parser_errors(parsed).any(|error| matches!(
                error,
                ParserErrorType::DeclarationSpecifierNotAllowedHere(_)
            )));
            assert_eq!(parsed.items.len(), 2, "{specifier}: {:#?}", parsed.items);
            assert!(parsed.parser.frames.is_empty());
            assert!(parsed.parser.returned.is_none());
        });
    }

    with_parse("struct { typedef", |truncated| {
        assert!(truncated.parser.frames.is_empty());
        assert!(truncated.parser.returned.is_none());
    });
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(96))]

    #[test]
    fn bounded_token_shaped_input_never_panics_or_leaks_state(
        pieces in prop::collection::vec(
            prop_oneof![
                Just("int"), Just("x"), Just("typedef"), Just("return"),
                Just("if"), Just("else"), Just("for"), Just("struct"),
                Just("enum"), Just("("), Just(")"), Just("["), Just("]"),
                Just("{"), Just("}"), Just(";"), Just(","), Just(":"),
                Just("?"), Just("="), Just("+"), Just("*"), Just("0")
            ],
            0..80,
        )
    ) {
        let source = pieces.join(" ");
        with_parse(&source, |parsed| {
            prop_assert!(parsed.parser.frames.is_empty());
            prop_assert!(parsed.parser.returned.is_none());
            prop_assert!(parsed.parser.recovery.active.is_none());
            prop_assert_eq!(parsed.parser.scopes.depth(), 0);
            prop_assert!(parsed.parser.label_scopes.is_empty());
            prop_assert!(parsed.parser.switch_scopes.is_empty());
            for error in &parsed.errors {
                let vectors = error.source_vectors(parsed.context);
                prop_assert!(vectors.length > 0 || source.is_empty());
            }
            Ok(())
        })?;
    }
}

#[test]
fn pending_preprocessing_diagnostics_survive_arena_compaction() {
    fn preprocessing_vectors(parsed: &Parsed<'_, '_>) -> Vec<SourceVector> {
        parsed
            .errors
            .iter()
            .filter_map(|error| match error {
                | TranslationError::Preprocessing(error) => Some(error.source_vectors),
                | _ => None,
            })
            .flat_map(|source| parsed.context.get_source_vectors(source).to_vec())
            .collect()
    }

    let short_vectors = with_parse("#undef\nint first;\n", |parsed| {
        preprocessing_vectors(parsed)
    });
    with_parse(
        &format!("#undef\nint first;\n{}", "int later;\n".repeat(200)),
        |long| {
            assert_ne!(short_vectors, []);
            assert_eq!(short_vectors, preprocessing_vectors(long));
            assert!(
                long.context.source_vectors.0.len() < 16,
                "the parser left {} vectors in the preprocessor arena",
                long.context.source_vectors.0.len()
            );
        },
    );
}
