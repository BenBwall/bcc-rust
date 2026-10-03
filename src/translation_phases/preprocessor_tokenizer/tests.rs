//! Differential tests: the batch lexing strategy must be observably identical
//! to the streaming one, token for token and diagnostic for diagnostic.

use std::path::{
    Path,
    PathBuf,
};

use proptest::prelude::*;

use super::{
    LexingStrategy,
    TokenSource,
};
use crate::{
    cli::describe_token,
    diagnostics::{
        ColorChoice as RenderColor,
        Renderer,
        ToDiagnostic,
    },
    translation_phases::{
        Context,
        GetPosition,
        GetSourceVectors,
        SetPosition,
        SourcePosition,
        SourceVector,
        TranslationError,
        TranslationPhase,
        preprocessing::Preprocessor,
    },
    util::shared::{
        SharedString,
        SharedVec,
    },
};

/// Renders every pending diagnostic while its provenance is still live.
fn drain_diagnostics(context: &mut Context, events: &mut Vec<String>) {
    while let Some(error) = context.pop_pending_error() {
        let source = error.source_vectors(context);
        let rendered = Renderer::new(RenderColor::Plain)
            .render(&error.to_diagnostic(context, source), context);
        events.push(format!(
            "diagnostic {:?}\n{rendered}",
            context.get_source_vectors(source)
        ));
    }
}

/// One step of a scripted walk over a token source.
#[derive(Clone, Debug)]
enum Step {
    Next,
    /// Reads the next token as an `#include` operand.
    NextHeader,
    Save,
    /// Rewinds to the most recent saved position.
    Restore,
    /// Renumbers the current line, as `#line` does.
    SetLine(u32),
}

/// Applies `steps` and then reads to the end, recording every observable
/// effect.
fn walk(source: &str, strategy: LexingStrategy, steps: &[Step]) -> Vec<String> {
    let mut context = Context::new();
    context.set_lexing_strategy(strategy);
    let file = context.intern_source_file(PathBuf::from("<test>").into_boxed_path());
    let source = SharedString::from(source.to_owned());
    context.record_source_text(file, source.clone());
    let mut tokens = TokenSource::new(&mut context, file, source);
    let mut saved: Vec<SourcePosition> = Vec::new();
    let mut events = Vec::new();
    let read = |tokens: &mut TokenSource, context: &mut Context, events: &mut Vec<String>| {
        let token = tokens.next_item(context);
        drain_diagnostics(context, events);
        events.push(match token {
            | Some(token) => format!(
                "{:?} {:?} {:?} include={}",
                token.kind,
                context.string_cache.at(token.contents),
                context.get_source_vectors(token.source_vectors),
                context.is_tokenizing_include_string(),
            ),
            | None => "end".to_owned(),
        });
        events.push(format!("at {:?}", tokens.position(context)));
        token.is_some()
    };
    for step in steps {
        match step {
            | Step::Next => _ = read(&mut tokens, &mut context, &mut events),
            | Step::NextHeader => {
                context.set_is_tokenizing_include_string(true);
                _ = read(&mut tokens, &mut context, &mut events);
                context.set_is_tokenizing_include_string(false);
            },
            | Step::Save => saved.push(tokens.position(&context)),
            | Step::Restore =>
                if let Some(&position) = saved.last() {
                    tokens.set_position(&mut context, position);
                },
            | Step::SetLine(line) => tokens.set_line(&mut context, *line),
        }
    }
    let mut remaining = 0;
    while read(&mut tokens, &mut context, &mut events) {
        remaining += 1;
        assert!(remaining < 10_000, "token sources must reach the end");
    }
    events
}

fn assert_same_tokens(source: &str, steps: &[Step]) {
    let streaming = walk(source, LexingStrategy::Streaming, steps);
    let batch = walk(source, LexingStrategy::Batch, steps);
    pretty_assertions::assert_eq!(streaming, batch, "source: {source:?}, steps: {steps:?}");
}

#[test]
fn ordinary_tokens_match() {
    for source in [
        "int x = 42;\n",
        "a+=b->c<<=d>>=e...f.5e+3 1.e-2 0x1P-4 .x\n",
        "L\"wide\" L'w' Lx \"s\\\"q\" 'c' '\\''\n",
        "#define F(a, ...) a ## __VA_ARGS__ #a\n",
        "<: :> <% %> %: %:%: %:% %= <<= >>= && || != == ! ~ ^= |= &= ? ;\n",
        "é𝑥 = ünïcode; € \u{a0}x\n",
        "  \t\x0b\x0c x \u{a0}\u{2003} y\n",
        "",
        "\n\n\n",
    ] {
        assert_same_tokens(source, &[]);
    }
}

#[test]
fn comments_match() {
    for source in [
        "a /* b */ c // d\ne\n",
        "#define X 1 // comment\nX\n",
        "char *s = \"a//b\"; char c = '/*';\n",
        "x /* multi\nline\r\ncomment */ y\n",
        "/**/ /***/ /* ** / */z\n",
        "/* unterminated",
        "/* unterminated\n",
        "// no newline",
        "a/\\\n* spliced comment *\\\n/b\n",
        "a ??/\n// trigraph spliced comment\nb\n",
        "/*??/\n",
    ] {
        assert_same_tokens(source, &[]);
    }
}

#[test]
fn phase_one_and_two_rewrites_match() {
    for source in [
        "??=define ??( ??) ??< ??> ??' ??! ??- ??/ ???= ?? ?\n",
        "ab\\\ncd \\\r\nef\\\rgh\n",
        "line1\r\nline2\rline3\n",
        "x ??/\ny\n",
        "\"str\\\ning\" 'a\\\n'\n",
        "trailing\\\n",
        "trailing splice\\\n\\\n",
        "x\\",
        "\\ $ @ `\n",
        "a ??/ b\n",
        "/**/1e\\\n",
    ] {
        assert_same_tokens(source, &[]);
    }
}

#[test]
fn literal_errors_and_missing_newlines_match() {
    for source in [
        "\"unterminated\nx\n",
        "'unterminated\nx\n",
        "\"eof",
        "\"eof\\",
        "\"eof\\\\\n",
        "abc",
        "1e",
        "x;",
        "..",
        "a <",
        "#include <a.h>",
        "#include \"a.h\"",
    ] {
        assert_same_tokens(source, &[]);
    }
}

#[test]
fn header_names_match_in_include_mode() {
    let steps = [
        Step::Next,
        Step::Next,
        Step::NextHeader,
        Step::NextHeader,
        Step::Next,
    ];
    for source in [
        "#include <stdio.h>\n",
        "#include \"dir\\file.h\"\n",
        "#include <unterminated\nx\n",
        "#include \"unterminated\nx\n",
        "#include <a//b.h>\n",
        "#define H <x.h>\nH\n",
        "#  include<a.h>\n",
        "#include <a.h>",
    ] {
        assert_same_tokens(source, &steps);
    }
    // A header name in a skipped group is read as ordinary tokens.
    assert_same_tokens("#include <a'b.h>\n", &[]);
}

#[test]
fn rewinds_and_line_renumbering_match() {
    assert_same_tokens(
        "a b\\\nc d\n",
        &[
            Step::Next,
            Step::Save,
            Step::Next,
            Step::Next,
            Step::Restore,
            Step::Next,
        ],
    );
    assert_same_tokens(
        "#line 100\nx\ny\n",
        &[
            Step::Next,
            Step::Next,
            Step::Next,
            Step::SetLine(100),
            Step::Next,
            Step::Next,
            Step::Save,
            Step::Next,
            Step::Next,
            Step::Restore,
        ],
    );
    // Rewinding onto the supplied final newline after reading it must not
    // supply it again, and a header name lexed through to the end of input
    // must still be followed by it.
    assert_same_tokens("", &[Step::Next, Step::SetLine(1)]);
    assert_same_tokens("0<>", &[Step::Next, Step::NextHeader, Step::SetLine(1)]);
}

fn fragment() -> impl Strategy<Value = &'static str> {
    prop::sample::select(vec![
        "a", "L", "_x9", "é", "€", "\u{a0}", "0", "1e", "0x", "+", "-", ".", "..", "'", "\"", "\\",
        "\n", "\r", "\r\n", "?", "??=", "??/", "??(", "??", " ", "\t", "\x0c", "/", "*", "/*",
        "*/", "//", "#", "%", ":", "%:", "<", ">", "=", "&", "|", "!", "^", "~", ",", ";", "(",
        ")", "include", "#include", "$", "defined",
    ])
}

fn step() -> impl Strategy<Value = Step> {
    prop_oneof![
        4 => Just(Step::Next),
        1 => Just(Step::NextHeader),
        1 => Just(Step::Save),
        1 => Just(Step::Restore),
        1 => (1u32..50).prop_map(Step::SetLine),
    ]
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(3000))]

    #[test]
    fn random_sources_lex_identically(
        fragments in proptest::collection::vec(fragment(), 0..40),
        steps in proptest::collection::vec(step(), 0..12),
    ) {
        let source = fragments.concat();
        let streaming = walk(&source, LexingStrategy::Streaming, &steps);
        let batch = walk(&source, LexingStrategy::Batch, &steps);
        prop_assert_eq!(streaming, batch, "source: {:?}, steps: {:?}", source, steps);
    }
}

/// Runs phases 1 through 6 and records every parser-facing token and
/// diagnostic.
fn preprocess(
    source: &str,
    path: &Path,
    include_directory: Option<&Path>,
    strategy: LexingStrategy,
) -> Vec<String> {
    let mut context = Context::new();
    context.set_lexing_strategy(strategy);
    let directories: SharedVec<PathBuf> = include_directory
        .map(|directory| vec![directory.to_owned()])
        .unwrap_or_default()
        .into();
    let mut preprocessor = Preprocessor::new(
        &mut context,
        path.into(),
        source.to_owned().into(),
        directories.clone(),
        directories,
    );
    let mut events = Vec::new();
    loop {
        let token = preprocessor.next_item(&mut context);
        drain_diagnostics(&mut context, &mut events);
        let Some(token) = token else {
            break;
        };
        events.push(format!(
            "{} {:?}",
            describe_token(token, &context),
            context.get_source_vectors(token.source_vectors)
        ));
    }
    events
}

fn assert_same_preprocessing(source: &str, path: &Path, include_directory: Option<&Path>) {
    let streaming = preprocess(source, path, include_directory, LexingStrategy::Streaming);
    let batch = preprocess(source, path, include_directory, LexingStrategy::Batch);
    pretty_assertions::assert_eq!(streaming, batch, "source: {source:?}");
}

#[test]
fn test_programs_preprocess_identically() {
    let directory = Path::new(env!("CARGO_MANIFEST_DIR")).join("test-programs");
    let mut checked = 0;
    for entry in std::fs::read_dir(&directory).expect("test-programs is readable") {
        let path = entry.expect("directory entries are readable").path();
        if path.extension().is_some_and(|extension| extension == "c") {
            let source =
                crate::util::read_to_string_lossy(&path).expect("test program is readable");
            assert_same_preprocessing(&source, &path, Some(&directory));
            checked += 1;
        }
    }
    assert!(checked > 10, "the test-program corpus must be found");
}

#[test]
fn directives_and_macros_preprocess_identically() {
    for source in [
        "#define F(x, y) x ## y + #x\nF(a, b) F(, 1) F(\"s\", 'c')\n",
        "#define G(...) [__VA_ARGS__]\nG() G(1, (2, 3), 4)\n",
        "#define A B\n#define B A\nA B\n",
        "#if defined(A) || 1\nyes\n#elif 0\nno\n#else\nno\n#endif\n",
        "#if 0\n#include <a'b.h>\n\"unterminated\n#else\nok\n#endif\n",
        "#line 100 \"renamed.c\"\n__LINE__ __FILE__ x\n",
        "_Pragma(\"once\") after\n",
        "#define X 1 // comment\nX\n",
        "#define EMPTY\n#define S(x) #x\nS( a  /* c */ \"b\\n\" ) EMPTY\n",
        "#define H <stdio.h>\n#include H\n",
        "#define L(x) L ## x\nL(\"wide\") L('c')\n",
        "??=define T 2\nT ??/\n+ 1\n",
        "#error message here\nafter\n",
        "#define F(x) x\nF(\n#define Y 1\nY)\n",
        "#define FUNC(x) x\nFUNC\n(1) FUNC + 2\n",
        "#define X 1",
        "\"adjacent\" \"strings\" L\"wide\" \"mixed\"\n",
        "0x1fUL 1.5e+3f 'ab' L'x' 077 08\n",
        "#if 1\n",
        "#endif\n",
    ] {
        assert_same_preprocessing(source, Path::new("<test>"), None);
    }
}

fn program_line() -> impl Strategy<Value = &'static str> {
    prop::sample::select(vec![
        "#define A 1",
        "#define A 2",
        "#define F(x, y) x ## y + #x",
        "#define G(...) __VA_ARGS__",
        "#define H(x) F(x, x) G(x)",
        "#undef A",
        "#if A",
        "#ifdef F",
        "#ifndef G",
        "#elif 0",
        "#else",
        "#endif",
        "F(a, b)",
        "G(1, 2) H(z)",
        "A + A",
        "F(",
        ")",
        "\"str\" \"cat\"",
        "'c' L'w' L\"w\"",
        "x = 1.5e+3f;",
        "/* c */ y // d",
        "/* open",
        "close */",
        "#line 100",
        "#line 7 \"foo.c\"",
        "_Pragma(\"x\")",
        "__LINE__ __FILE__",
        "??=define T 2",
        "a\\",
        "#error oops",
        "#pragma x",
        "$ @",
        "\"unterminated",
        "#include",
        "#include <missing.h>",
    ])
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(500))]

    #[test]
    fn random_programs_preprocess_identically(
        lines in proptest::collection::vec(program_line(), 0..16),
        crlf in any::<bool>(),
        final_newline in any::<bool>(),
    ) {
        let mut source = lines.join(if crlf { "\r\n" } else { "\n" });
        if final_newline {
            source.push('\n');
        }
        let streaming = preprocess(&source, Path::new("<test>"), None, LexingStrategy::Streaming);
        let batch = preprocess(&source, Path::new("<test>"), None, LexingStrategy::Batch);
        prop_assert_eq!(streaming, batch, "source: {:?}", source);
    }
}

#[test]
fn physically_empty_source_has_no_missing_final_newline() {
    for strategy in [LexingStrategy::Streaming, LexingStrategy::Batch] {
        let mut context = Context::new();
        context.set_lexing_strategy(strategy);
        let file = context.intern_source_file(PathBuf::from("<test>").into_boxed_path());
        let source = SharedString::from(String::new());
        context.record_source_text(file, source.clone());
        let mut tokens = TokenSource::new(&mut context, file, source);
        assert!(tokens.next_item(&mut context).is_none(), "{strategy:?}");
        assert!(context.take_pending_errors().is_empty(), "{strategy:?}");
    }
    assert_same_tokens("", &[]);
}

#[test]
fn source_emptied_by_splicing_reports_escaped_final_newline() {
    for source in ["\\\n", "??/\n", "\\\r\n", "\\\n\\\n"] {
        for strategy in [LexingStrategy::Streaming, LexingStrategy::Batch] {
            let events = walk(source, strategy, &[]);
            assert!(
                events
                    .iter()
                    .any(|event| event.contains("final newline is escaped")),
                "{source:?}, {strategy:?}: {events:#?}",
            );
        }
        assert_same_tokens(source, &[]);
    }
}

#[test]
fn unterminated_block_comments_report_the_actual_opener() {
    for (source, index, line, column) in [
        ("/* broken\n", 0, 1, 1),
        ("  /* broken\n", 2, 1, 3),
        ("int sentinel; /* broken\n", 14, 1, 15),
        (" \\\n/* broken\n", 3, 2, 1),
        (" /\\\n* broken\n", 1, 1, 2),
        (" /**/ /* broken\n", 6, 1, 7),
        ("/* broken", 0, 1, 1),
        ("/* broken\\\n", 0, 1, 1),
        ("/*", 0, 1, 1),
    ] {
        for strategy in [LexingStrategy::Streaming, LexingStrategy::Batch] {
            let mut context = Context::new();
            context.set_lexing_strategy(strategy);
            let file = context.intern_source_file(PathBuf::from("<test>").into_boxed_path());
            let text = SharedString::from(source.to_owned());
            context.record_source_text(file, text.clone());
            let mut tokens = TokenSource::new(&mut context, file, text);
            let mut spellings = Vec::new();
            while let Some(token) = tokens.next_item(&mut context) {
                spellings.push(context.string_cache.at(token.contents).to_owned());
            }
            let errors = context.take_pending_errors();
            let comments: Vec<_> = errors
                .iter()
                .filter_map(|error| match error {
                    | TranslationError::PreprocessorTokenizining(error)
                        if error.to_string() == "unterminated block comment" =>
                        Some(error),
                    | _ => None,
                })
                .collect();
            assert_eq!(comments.len(), 1, "{source:?}, {strategy:?}: {errors:#?}");
            let start = SourcePosition {
                index,
                line,
                column,
            };
            assert_eq!(
                comments[0].source_vector,
                SourceVector::new(start, file, source.len() - index),
                "{source:?}, {strategy:?}",
            );
            if source.starts_with("int") {
                assert!(spellings.iter().any(|spelling| spelling == "sentinel"));
            }
        }
        assert_same_tokens(source, &[]);
    }
}

#[test]
fn terminal_spliced_newlines_report_the_actual_last_splice() {
    for (source, index, line, column, length) in [
        ("\n\\\n", 1, 2, 1, 2),
        ("\n??/\n", 1, 2, 1, 4),
        ("\r\n\\\r\n", 2, 2, 1, 3),
        ("\r\n??/\r\n", 2, 2, 1, 5),
        ("\r\\\r", 1, 2, 1, 2),
        ("int x;\n\\\n", 7, 2, 1, 2),
        ("word\\\n", 4, 1, 5, 2),
        (" \\\n", 1, 1, 2, 2),
        ("\n\\\n\\\n", 3, 3, 1, 2),
        ("é\\\n", 2, 1, 2, 2),
    ] {
        for strategy in [LexingStrategy::Streaming, LexingStrategy::Batch] {
            let mut context = Context::new();
            context.set_lexing_strategy(strategy);
            let file = context.intern_source_file(PathBuf::from("<test>").into_boxed_path());
            let text = SharedString::from(source.to_owned());
            context.record_source_text(file, text.clone());
            let mut tokens = TokenSource::new(&mut context, file, text);
            while tokens.next_item(&mut context).is_some() {}
            let errors = context.take_pending_errors();
            assert_eq!(errors.len(), 1, "{source:?}: {strategy:?}: {errors:#?}");
            let source_vectors = errors[0].source_vectors(&mut context);
            assert_eq!(
                context.get_source_vectors(source_vectors),
                &[SourceVector::new(
                    SourcePosition {
                        index,
                        line,
                        column
                    },
                    file,
                    length,
                )]
            );
            assert_eq!(errors[0].to_string(), "final newline is escaped");
            assert!(tokens.next_item(&mut context).is_none());
            assert!(tokens.next_item(&mut context).is_none());
            assert!(
                context.take_pending_errors().is_empty(),
                "repeated EOF warns again"
            );
        }
        assert_same_tokens(source, &[]);
        assert_same_tokens(source, &[Step::Save, Step::Next, Step::Restore, Step::Next]);
        assert_same_tokens(source, &[Step::Next, Step::Save, Step::Next, Step::Restore]);
        assert_same_tokens(
            source,
            &[
                Step::SetLine(90),
                Step::Next,
                Step::Save,
                Step::Next,
                Step::Restore,
                Step::Next,
            ],
        );
    }
}

#[test]
fn terminal_splice_warning_is_deferred_until_the_tail_is_read() {
    // '\n' is already a complete token; batch construction must stay silent.
    for strategy in [LexingStrategy::Streaming, LexingStrategy::Batch] {
        let mut context = Context::new();
        context.set_lexing_strategy(strategy);
        let file = context.intern_source_file(PathBuf::from("<test>").into_boxed_path());
        let text = SharedString::from("\n\\\n".to_owned());
        context.record_source_text(file, text.clone());
        let mut tokens = TokenSource::new(&mut context, file, text);
        assert!(context.take_pending_errors().is_empty());
        let first = tokens.next_item(&mut context).unwrap();
        assert_eq!(context.string_cache.at(first.contents), "\n");
        assert!(context.take_pending_errors().is_empty());
        // The prior logical LF stays one token; EOF warns without adding one.
        assert!(tokens.next_item(&mut context).is_none());
        assert_eq!(context.take_pending_errors().len(), 1);
    }
}

#[test]
fn cloned_terminal_splice_cursors_keep_independent_warning_state() {
    for source in ["\n\\\n", "word\\\n"] {
        for strategy in [LexingStrategy::Streaming, LexingStrategy::Batch] {
            let mut context = Context::new();
            context.set_lexing_strategy(strategy);
            let file = context.intern_source_file(PathBuf::from("<test>").into_boxed_path());
            let text = SharedString::from(source.to_owned());
            context.record_source_text(file, text.clone());
            let mut original = TokenSource::new(&mut context, file, text);
            let mut cloned = original.clone();
            while original.next_item(&mut context).is_some() {}
            assert_eq!(context.take_pending_errors().len(), 1);
            while cloned.next_item(&mut context).is_some() {}
            assert_eq!(context.take_pending_errors().len(), 1);
            assert!(original.next_item(&mut context).is_none());
            assert!(cloned.next_item(&mut context).is_none());
            assert!(context.take_pending_errors().is_empty());
        }
    }
}
