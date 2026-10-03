//! Pipeline iterator regressions and crate-level syntax-tree consumption.

use std::path::PathBuf;

use super::*;
use crate::{
    configuration::{
        CStandard,
        CompilerConfiguration,
        ExtensionPolicy,
    },
    diagnostics::{
        ColorChoice as RenderColor,
        Renderer,
        ToDiagnostic,
    },
    translation_phases::{
        Context,
        GetSourceVectors,
        TranslationError,
        parsing::{
            ExternalDeclaration,
            Parser as LanguageParser,
        },
        preprocessing::{
            Preprocessor,
            PreprocessorError,
            PreprocessorErrorType,
            TokenType,
        },
    },
    util::shared::SharedVec,
};

#[test]
fn diagnostic_is_yielded_before_the_token_produced_alongside_it() {
    let mut iterator = PreprocessorIterator::new(
        PathBuf::from("<test>").into_boxed_path(),
        "#if (0, 2)\nCOMMA_RESULT_2\n#endif\n".to_owned().into(),
        SharedVec::default(),
        SharedVec::default(),
    );
    iterator.context.configuration =
        CompilerConfiguration::new(CStandard::C99, ExtensionPolicy::Deny);

    let error = iterator.next().unwrap().unwrap_err();
    assert!(matches!(
        &error,
        TranslationError::Preprocessing(PreprocessorError {
            error_type: PreprocessorErrorType::CommaOperatorInPreprocessorExpression(
                ExtensionPolicy::Deny
            ),
            ..
        })
    ));
    let error_source_vectors = error.source_vectors(&mut iterator.context);
    assert_ne!(
        iterator.context.get_source_vectors(error_source_vectors),
        []
    );

    let token = iterator.next().unwrap().unwrap();
    assert_eq!(token.kind, TokenType::Identifier);
    assert_eq!(
        iterator.context.string_cache.at(token.contents),
        "COMMA_RESULT_2"
    );
    assert!(iterator.next().is_none());
}

#[test]
fn nested_token_pastes_keep_their_operands_while_tokens_are_yielded() {
    let mut iterator = PreprocessorIterator::new(
        PathBuf::from("<test>").into_boxed_path(),
        concat!(
            "#define LIM1(x) x##0; x##1;\n",
            "#define LIM2(x) LIM1(x##0) LIM1(x##1)\n",
            "#define LIM3(x) LIM2(x##0) LIM2(x##1)\n",
            "LIM3(int value)\n",
        )
        .to_owned()
        .into(),
        SharedVec::default(),
        SharedVec::default(),
    );

    let mut names = Vec::new();
    while let Some(token) = iterator.next() {
        let token = token.unwrap();
        assert_ne!(
            iterator.context.get_source_vectors(token.source_vectors),
            []
        );
        if token.kind == TokenType::Identifier {
            names.push(iterator.context.string_cache.at(token.contents).to_owned());
        }
    }
    assert_eq!(
        names,
        [
            "value000", "value001", "value010", "value011", "value100", "value101", "value110",
            "value111",
        ]
    );
}

#[test]
fn adjacent_string_lookahead_keeps_the_buffered_token_provenance() {
    let mut iterator = PreprocessorIterator::new(
        PathBuf::from("<test>").into_boxed_path(),
        "\"a\" identifier\n".to_owned().into(),
        SharedVec::default(),
        SharedVec::default(),
    );

    let string = iterator.next().unwrap().unwrap();
    assert!(matches!(string.kind, TokenType::String(_)));
    let identifier = iterator.next().unwrap().unwrap();
    assert_eq!(identifier.kind, TokenType::Identifier);
    assert_eq!(
        iterator.context.string_cache.at(identifier.contents),
        "identifier"
    );
    assert_ne!(
        iterator
            .context
            .get_source_vectors(identifier.source_vectors),
        []
    );
    assert!(iterator.next().is_none());
}

#[test]
fn adjacent_string_lookahead_defers_buffered_token_diagnostics() {
    let mut iterator = PreprocessorIterator::new(
        PathBuf::from("<test>").into_boxed_path(),
        "\"a\" 0xg\n".to_owned().into(),
        SharedVec::default(),
        SharedVec::default(),
    );

    assert!(matches!(
        iterator.next().unwrap().unwrap().kind,
        TokenType::String(_)
    ));
    assert!(matches!(
        iterator.next().unwrap().unwrap_err(),
        TranslationError::Preprocessing(PreprocessorError {
            error_type: PreprocessorErrorType::InvalidHexadecimalIntegerLiteral,
            ..
        })
    ));
    assert!(iterator.next().is_some());
    assert!(iterator.next().is_none());
}

#[test]
fn adjacent_string_lookahead_keeps_current_token_before_later_diagnostics() {
    let mut iterator = PreprocessorIterator::new(
        PathBuf::from("<test>").into_boxed_path(),
        "\"\\q\" 0xg".to_owned().into(),
        SharedVec::default(),
        SharedVec::default(),
    );

    assert!(matches!(
        iterator.next().unwrap().unwrap_err(),
        TranslationError::Preprocessing(PreprocessorError {
            error_type: PreprocessorErrorType::InvalidEscapeSequence,
            ..
        })
    ));
    assert!(matches!(
        iterator.next().unwrap().unwrap().kind,
        TokenType::String(_)
    ));
    assert!(matches!(
        iterator.next().unwrap().unwrap_err(),
        TranslationError::InitialProcessing(_)
    ));
    assert!(matches!(
        iterator.next().unwrap().unwrap_err(),
        TranslationError::Preprocessing(PreprocessorError {
            error_type: PreprocessorErrorType::InvalidHexadecimalIntegerLiteral,
            ..
        })
    ));
    assert!(matches!(
        iterator.next().unwrap().unwrap().kind,
        TokenType::Integer(_)
    ));
    assert!(iterator.next().is_none());
}

#[test]
fn adjacent_string_lookahead_keeps_deferred_eof_diagnostic_provenance() {
    let mut iterator = PreprocessorIterator::new(
        PathBuf::from("<test>").into_boxed_path(),
        "\"a\"\n#error boom\n".to_owned().into(),
        SharedVec::default(),
        SharedVec::default(),
    );

    assert!(matches!(
        iterator.next().unwrap().unwrap().kind,
        TokenType::String(_)
    ));
    let error = iterator.next().unwrap().unwrap_err();
    assert!(matches!(
        &error,
        TranslationError::Preprocessing(PreprocessorError {
            error_type: PreprocessorErrorType::ErrorDirective(message),
            ..
        }) if message.trim() == "boom"
    ));
    let source_vectors = error.source_vectors(&mut iterator.context);
    assert_ne!(iterator.context.get_source_vectors(source_vectors), []);
    assert!(iterator.next().is_none());
}

#[test]
fn parser_iterator_yields_declarations_and_exposes_the_syntax_store() {
    let mut iterator = ParserIterator::new(
        PathBuf::from("<test>").into_boxed_path(),
        "int value;\n".to_owned().into(),
        SharedVec::default(),
        SharedVec::default(),
    );

    assert!(matches!(
        iterator.next().unwrap().unwrap(),
        ExternalDeclaration::Declaration(_)
    ));
    assert!(iterator.next().is_none());
    assert!(
        format!("{:#?}", iterator.parser.syntax_debug()).contains("declarations:"),
        "the debug view should expose the arena referenced by parser output"
    );
}

#[test]
fn parser_iterator_yields_a_parsed_initialized_declaration() {
    let mut iterator = ParserIterator::new(
        PathBuf::from("<test>").into_boxed_path(),
        "int value = 1;\n".to_owned().into(),
        SharedVec::default(),
        SharedVec::default(),
    );

    assert!(matches!(
        iterator.next().unwrap().unwrap(),
        ExternalDeclaration::Declaration(_)
    ));
    assert!(iterator.next().is_none());
}

#[test]
fn complete_translation_unit_owns_ordered_roots_and_typed_syntax() {
    let mut context = Context::new();
    let preprocessor = Preprocessor::new(
        &mut context,
        PathBuf::from("<test>").into_boxed_path(),
        "int first; int second = 2;\n".to_owned().into(),
        SharedVec::default(),
        SharedVec::default(),
    );

    let unit = LanguageParser::new(preprocessor).parse_translation_unit(&mut context);

    assert_eq!(unit.external_declarations().len(), 2);
    let ExternalDeclaration::Declaration(first) = unit.external_declarations()[0] else {
        panic!("expected the first root to be a declaration")
    };
    let ExternalDeclaration::Declaration(second) = unit.external_declarations()[1] else {
        panic!("expected the second root to be a declaration")
    };
    assert_eq!(unit.syntax().declaration(first).init_declarators().len(), 1);
    assert_eq!(
        unit.syntax().declaration(second).init_declarators().len(),
        1
    );
}

#[test]
fn cli_parser_details_render_recovery_ranges_and_notes() {
    let mut context = Context::new();
    let preprocessor = Preprocessor::new(
        &mut context,
        PathBuf::from("<test>").into_boxed_path(),
        "int first extra junk; int after;\n".to_owned().into(),
        SharedVec::default(),
        SharedVec::default(),
    );
    let _unit = LanguageParser::new(preprocessor).parse_translation_unit(&mut context);
    let errors = context.take_pending_errors();
    let diagnostic = errors
        .iter()
        .find_map(|error| match error {
            | TranslationError::Parsing(error)
                if error
                    .recovery
                    .is_some_and(|recovery| recovery.discarded_tokens > 0) =>
                Some(error),
            | _ => None,
        })
        .expect("expected discarded-input recovery");

    let source = diagnostic.source_vectors(&mut context);
    let rendered = Renderer::new(RenderColor::Plain)
        .render(&diagnostic.to_diagnostic(&context, source), &context);
    let expected = [
        "error: expected `,`, `=`, `;`, or a function body after the declarator, found identifier \
         `extra`",
        " --> <test>:1:11",
        "  |",
        "1 | int first extra junk; int after;",
        "  |           ^^^^^ ---- skipped to recover",
        "  |           |",
        "  |           expected one of `,`, `=`, `;`, or `{`",
        "  |",
        "  = help: if this starts a new declaration, add `;` before it",
        "",
        "",
    ]
    .join("\n");
    assert_eq!(rendered, expected);
}
use crate::translation_phases::parsing::{
    DirectDeclarator,
    TypeSpecifiers,
};

#[test]
fn sibling_consumer_can_traverse_parameter_and_member_syntax() {
    let mut context = Context::new();
    let preprocessor = Preprocessor::new(
        &mut context,
        PathBuf::from("<syntax-tree-consumer-test>").into_boxed_path(),
        "struct S { int member : 3; }; int f(int parameter);"
            .to_owned()
            .into(),
        SharedVec::default(),
        SharedVec::default(),
    );
    let unit = LanguageParser::new(preprocessor).parse_translation_unit(&mut context);
    let tree = unit.syntax();

    let ExternalDeclaration::Declaration(struct_root) = unit.external_declarations()[0] else {
        panic!("expected struct declaration")
    };
    let TypeSpecifiers::StructOrUnion(struct_index) = tree
        .declaration(struct_root)
        .syntax()
        .declaration_specifiers
        .type_specifiers
    else {
        panic!("expected struct type specifier")
    };
    let members = tree.struct_declarations(
        tree.struct_or_union_specifier(struct_index)
            .struct_declaration_list
            .expect("struct definition has members"),
    );
    assert_eq!(members[0].type_specifiers, TypeSpecifiers::Int);
    let member_declarators = tree.struct_declarators(members[0].struct_declarator_list);
    assert!(member_declarators[0].declarator.is_some());
    assert!(member_declarators[0].bitfield_width.is_some());

    let ExternalDeclaration::Declaration(function_root) = unit.external_declarations()[1] else {
        panic!("expected function declaration")
    };
    let declarator = tree.declaration(function_root).init_declarators()[0].declarator;
    let parameter_list = tree
        .direct_declarators(declarator.kind)
        .iter()
        .find_map(|direct| match direct {
            | DirectDeclarator::Function { parameter_list, .. } => Some(*parameter_list),
            | _ => None,
        })
        .expect("function declarator has a parameter list");
    let parameters = tree.parameter_declarations(parameter_list);
    assert_eq!(
        parameters[0].declaration_specifiers.type_specifiers,
        TypeSpecifiers::Int
    );
    assert!(parameters[0].declarator.is_some());
}
