//! Required C99 predefined macros.

use std::path::PathBuf;

use super::Preprocessor;
use crate::{
    translation_phases::{
        Context,
        SourceVector,
        preprocessing::{
            IntegerTokenType,
            Token,
            TokenType,
        },
    },
    util::{
        packed::Packed,
        shared::SharedVec,
    },
};

#[derive(Debug, Default)]
struct Observation {
    kinds:     Vec<TokenType>,
    spellings: Vec<String>,
    sources:   Vec<Vec<SourceVector>>,
    errors:    Vec<String>,
}

fn record_token(token: Token, context: &Context<'_>, observation: &mut Observation) {
    observation.kinds.push(token.kind);
    observation.spellings.push(
        context
            .string_cache
            .at(token.contents)
            .trim_end_matches('\0')
            .to_owned(),
    );
    observation
        .sources
        .push(context.get_source_vectors(token.source_vectors).to_vec());
}

fn record_errors(context: &mut Context<'_>, observation: &mut Observation) {
    observation.errors.extend(
        context
            .take_pending_errors()
            .into_iter()
            .map(|error| error.to_string()),
    );
}

fn observe(source: &str) -> Observation {
    let tu = crate::util::bump::Bump::new();
    let mut context = Context::new(&tu);
    let tok = crate::util::bump::Bump::new();
    let preprocess_arena = crate::util::bump::Bump::new();
    let mut preprocessor = Preprocessor::new(
        &preprocess_arena,
        &mut context,
        PathBuf::from("<predefined regressions>").into_boxed_path(),
        source,
        SharedVec::default(),
        SharedVec::default(),
    );
    let mut observation = Observation::default();
    for token in preprocessor.preprocess_all(&mut context, &tok) {
        record_token(token, &context, &mut observation);
    }
    record_errors(&mut context, &mut observation);
    observation
}

fn identifier_spellings(observation: &Observation) -> Vec<&str> {
    observation
        .kinds
        .iter()
        .zip(&observation.spellings)
        .filter_map(|(kind, spelling)| {
            matches!(kind, TokenType::Identifier).then_some(spelling.as_str())
        })
        .collect()
}

fn integer_kinds(observation: &Observation) -> Vec<IntegerTokenType> {
    observation
        .kinds
        .iter()
        .filter_map(|kind| match kind {
            | TokenType::Integer(value) => Some(*value),
            | _ => None,
        })
        .collect()
}

#[test]
fn standard_macros_expand_to_required_numeric_types_and_values() {
    let source = "__STDC__; __STDC_VERSION__; __STDC_HOSTED__; __STDC_MB_MIGHT_NEQ_WC__; after\n";
    let observation = observe(source);
    assert!(observation.errors.is_empty(), "{observation:#?}");
    assert_eq!(
        integer_kinds(&observation),
        [
            IntegerTokenType::Int(1),
            IntegerTokenType::Long(Packed::new(199_901)),
            IntegerTokenType::Int(0),
            IntegerTokenType::Int(1),
        ]
    );
    assert_eq!(
        observation.spellings,
        ["1", ";", "199901L", ";", "0", ";", "1", ";", "after"]
    );
    for (index, column, length) in [(0, 1, 8), (2, 11, 16), (4, 29, 15), (6, 46, 24)] {
        let source = &observation.sources[index][0];
        assert_eq!(
            (source.line, source.column, source.length),
            (1, column, length)
        );
    }
}

#[test]
fn required_standard_macros_are_available_to_both_defined_forms() {
    let source = "#if defined __STDC__ && defined(__STDC_VERSION__) && defined(__STDC_HOSTED__) \
                  && defined(__STDC_MB_MIGHT_NEQ_WC__)\nall_defined\n#endif\nafter\n";
    let observation = observe(source);
    assert_eq!(identifier_spellings(&observation), ["all_defined", "after"]);
    assert!(observation.errors.is_empty(), "{observation:#?}");
}

#[test]
fn required_standard_macros_are_available_to_ifdef() {
    let source = "#ifdef __STDC__\nstdc_defined\n#endif\n#ifdef \
                  __STDC_VERSION__\nversion_defined\n#endif\n#ifdef \
                  __STDC_HOSTED__\nhosted_defined\n#endif\n#ifdef \
                  __STDC_MB_MIGHT_NEQ_WC__\nencoding_defined\n#endif\nafter\n";
    let observation = observe(source);
    assert_eq!(
        identifier_spellings(&observation),
        [
            "stdc_defined",
            "version_defined",
            "hosted_defined",
            "encoding_defined",
            "after"
        ]
    );
    assert!(observation.errors.is_empty(), "{observation:#?}");
}

#[test]
fn standard_macro_values_select_c99_freestanding_branch() {
    let source = "#if __STDC__ == 1 && __STDC_VERSION__ == 199901L && __STDC_HOSTED__ == 0 && \
                  __STDC_MB_MIGHT_NEQ_WC__ == \
                  1\nright_values\n#else\nwrong_values\n#endif\nafter\n";
    let observation = observe(source);
    assert_eq!(
        identifier_spellings(&observation),
        ["right_values", "after"]
    );
    assert!(observation.errors.is_empty(), "{observation:#?}");
}

#[test]
fn standard_version_expands_inside_an_ordinary_macro_alias() {
    let source = "#define VERSION_ALIAS __STDC_VERSION__\n#define ENCODING_ALIAS \
                  __STDC_MB_MIGHT_NEQ_WC__\nVERSION_ALIAS; ENCODING_ALIAS; after\n";
    let observation = observe(source);
    assert_eq!(
        integer_kinds(&observation),
        [
            IntegerTokenType::Long(Packed::new(199_901)),
            IntegerTokenType::Int(1)
        ]
    );
    assert_eq!(identifier_spellings(&observation), ["after"]);
    assert!(observation.errors.is_empty(), "{observation:#?}");
}

#[test]
fn ordinary_macro_definitions_and_undef_remain_usable() {
    let source = "#define STDC 7\n#define STDC_VERSION 8\n#define STDC_HOSTED 9\n#define \
                  STDC_MB_MIGHT_NEQ_WC 10\n#if defined(STDC) && STDC == 7 && STDC_VERSION == 8 && \
                  STDC_HOSTED == 9 && STDC_MB_MIGHT_NEQ_WC == 10\nnormal_defined\n#endif\nSTDC; \
                  STDC_VERSION; STDC_HOSTED; STDC_MB_MIGHT_NEQ_WC;\n#undef STDC\n#ifdef \
                  STDC\nunreachable\n#else\nnormal_undef\n#endif\nSTDC after\n";
    let observation = observe(source);
    assert_eq!(
        integer_kinds(&observation),
        [
            IntegerTokenType::Int(7),
            IntegerTokenType::Int(8),
            IntegerTokenType::Int(9),
            IntegerTokenType::Int(10)
        ]
    );
    assert_eq!(
        identifier_spellings(&observation),
        ["normal_defined", "normal_undef", "STDC", "after"]
    );
    assert!(observation.errors.is_empty(), "{observation:#?}");
}
