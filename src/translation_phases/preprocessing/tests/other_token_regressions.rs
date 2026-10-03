//! Non-token characters remain preprocessing tokens until phase 7.

use std::path::PathBuf;

use super::strategies::assert_strategies_agree;
use crate::{
    diagnostics::ToDiagnostic,
    pipeline::PreprocessingStrategy,
    translation_phases::{
        Context,
        GetPosition,
        GetSourceVectors,
        SourcePosition,
        SourceVector,
        TranslationPhase,
        preprocessing::{
            StringTokenType,
            Token,
            TokenType,
        },
    },
    util::shared::SharedVec,
};

const STRATEGIES: [PreprocessingStrategy; 3] = [
    PreprocessingStrategy::Streaming,
    PreprocessingStrategy::BatchLexing,
    PreprocessingStrategy::Batch,
];

#[derive(Debug)]
struct Observation {
    tokens: Vec<(String, Vec<SourceVector>)>,
    errors: Vec<(String, Vec<SourceVector>)>,
}

fn record_token(token: Token, context: &Context, observation: &mut Observation) {
    let spelling = match token.kind {
        | TokenType::String(StringTokenType::String(contents)) => format!(
            "string:{}",
            context
                .literal_text(contents, false)
                .as_deref()
                .expect("UTF-8 test literal")
        ),
        | _ => context.string_cache.at(token.contents).to_owned(),
    };
    observation.tokens.push((
        spelling,
        context.get_source_vectors(token.source_vectors).to_vec(),
    ));
}

fn record_errors(context: &mut Context, observation: &mut Observation) {
    while let Some(error) = context.pop_pending_error() {
        let sources = error.source_vectors(context);
        let message = error.to_diagnostic(context, sources).message;
        observation
            .errors
            .push((message, context.get_source_vectors(sources).to_vec()));
    }
}

fn observe(source: &str, strategy: PreprocessingStrategy) -> Observation {
    let mut context = Context::new();
    let mut preprocessor = strategy.preprocessor(
        &mut context,
        PathBuf::from("<other tokens>").into_boxed_path(),
        source.to_owned().into(),
        SharedVec::default(),
        SharedVec::default(),
    );
    let mut observation = Observation {
        tokens: Vec::new(),
        errors: Vec::new(),
    };
    if strategy == PreprocessingStrategy::Batch {
        let tokens = preprocessor.preprocess_all(&mut context);
        record_errors(&mut context, &mut observation);
        for token in tokens {
            record_token(token, &context, &mut observation);
        }
    } else {
        loop {
            let token = preprocessor.next_iterator_item(&mut context);
            record_errors(&mut context, &mut observation);
            let Some(token) = token else { break };
            record_token(token, &context, &mut observation);
        }
    }
    observation
}

#[test]
fn lexers_preserve_other_tokens_before_preprocessing() {
    for strategy in STRATEGIES {
        let mut context = Context::new();
        let mut preprocessor = strategy.preprocessor(
            &mut context,
            PathBuf::from("<other tokens>").into_boxed_path(),
            "a\\\n@\n".to_owned().into(),
            SharedVec::default(),
            SharedVec::default(),
        );
        let mut spellings = Vec::new();
        while let Some(token) = preprocessor.tokenizer.next_item(&mut context) {
            spellings.push(context.string_cache.at(token.contents).to_owned());
        }
        assert_eq!(spellings, ["a", "@", "\n"], "{strategy:?}");
        assert!(context.take_pending_errors().is_empty(), "{strategy:?}");
    }
}

#[test]
fn stringification_preserves_other_preprocessing_tokens() {
    for (argument, expected) in [("@", "@"), (": @", ": @"), ("$ `", "$ `")] {
        let source = format!("#define S(x) #x\nS({argument}); after\n");
        assert_strategies_agree(&source);
        for strategy in STRATEGIES {
            let actual = observe(&source, strategy);
            assert!(actual.errors.is_empty(), "{strategy:?}: {actual:#?}");
            assert_eq!(
                actual.tokens[0].0,
                format!("string:{expected}"),
                "{strategy:?}"
            );
            assert_eq!(actual.tokens.last().unwrap().0, "after", "{strategy:?}");
        }
    }
}

#[test]
fn unused_other_preprocessing_tokens_do_not_diagnose() {
    for source in [
        "#define UNUSED @\nafter\n",
        "#define DISCARD(x) after\nDISCARD(@)\n",
        "#if 0\n@ $ ` ??/ ;\n#endif\nafter\n",
    ] {
        assert_strategies_agree(source);
        for strategy in STRATEGIES {
            let actual = observe(source, strategy);
            assert!(
                actual.errors.is_empty(),
                "{strategy:?}: {source:?}: {actual:#?}"
            );
            assert_eq!(
                actual
                    .tokens
                    .iter()
                    .map(|token| token.0.as_str())
                    .collect::<Vec<_>>(),
                ["after"]
            );
        }
    }
}

#[test]
fn surviving_other_tokens_keep_their_actual_character_locations() {
    for (source, character, index, line, column, length) in [
        ("@ after\n", '@', 0, 1, 1, 1),
        ("$ after\n", '$', 0, 1, 1, 1),
        ("` after\n", '`', 0, 1, 1, 1),
        ("\u{1} after\n", '\u{1}', 0, 1, 1, 1),
        ("\\\n@ after\n", '@', 2, 2, 1, 1),
        ("a\\\n@ after\n", '@', 3, 2, 1, 1),
        ("a??/\n??/ after\n", '\\', 5, 2, 1, 3),
        ("☃ after\n", '☃', 0, 1, 1, 3),
        ("🦀 after\n", '🦀', 0, 1, 1, 4),
        ("#define BAD @\nBAD after\n", '@', 12, 1, 13, 1),
    ] {
        assert_strategies_agree(source);
        for strategy in STRATEGIES {
            let actual = observe(source, strategy);
            assert_eq!(
                actual.errors.len(),
                1,
                "{strategy:?}: {source:?}: {actual:#?}"
            );
            let (message, vectors) = &actual.errors[0];
            let quoted = match character {
                | '`' => "'`'".to_owned(),
                | c if c.is_control() => format!("U+{:04X}", u32::from(c)),
                | c => format!("`{c}`"),
            };
            assert_eq!(message, &format!("unexpected character {quoted} in source"));
            assert_eq!(vectors.len(), 1, "{strategy:?}: {vectors:#?}");
            assert_eq!(
                vectors[0].position(&Context::new()),
                SourcePosition {
                    index,
                    line,
                    column
                }
            );
            assert_eq!(vectors[0].length, length);
            assert_eq!(actual.tokens.last().unwrap().0, "after");
            if source.starts_with('a') {
                assert_eq!(actual.tokens[0].0, "a");
                assert_eq!(
                    actual.tokens[0].1[0].length, 1,
                    "previous token boundary moved"
                );
            }
        }
    }
}
