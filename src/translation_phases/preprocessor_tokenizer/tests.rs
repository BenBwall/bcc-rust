//! Lexer and preprocessor regression tests. Most compare every observable
//! effect of a scripted walk (tokens, provenance, positions, and rendered
//! diagnostics) with a snapshot under `tests/fixtures/lexing/`; run with
//! `BLESS=1` to rewrite them. The snapshots were first produced by the
//! on-demand lexer, the differential oracle the batch lexer replaced.

use std::path::{
    Path,
    PathBuf,
};

use proptest::prelude::*;

use super::{
    LexedFiles,
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
fn drain_diagnostics(context: &mut Context<'_>, events: &mut Vec<String>) {
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
    Save,
    /// Rewinds to the most recent saved position.
    Restore,
    /// Renumbers the current line, as `#line` does.
    SetLine(u32),
}

/// Applies `steps` and then reads to the end, recording every observable
/// effect.
fn walk(source: &str, steps: &[Step]) -> Vec<String> {
    let tu = crate::util::bump::Bump::new();
    let mut context = Context::new(&tu);
    let file = context.intern_source_file(Path::new("<test>"));
    let source = SharedString::from(source.to_owned());
    context.record_source_text(file, &source);
    // Lexing takes the rest of its arena, so it gets one of its own, as in
    // the preprocessor.
    let pp = crate::util::bump::Bump::new();
    let mut tokens = TokenSource::new(&mut context, &pp, file, &source);
    let mut saved: Vec<SourcePosition> = Vec::new();
    let mut events = Vec::new();
    let read =
        |tokens: &mut TokenSource<'_>, context: &mut Context<'_>, events: &mut Vec<String>| {
            let token = tokens.next_item(context);
            drain_diagnostics(context, events);
            events.push(match token {
                | Some(token) => format!(
                    "{:?} {:?} {:?}",
                    token.kind,
                    context.string_cache.at(token.contents),
                    context.get_source_vectors(token.source_vectors),
                ),
                | None => "end".to_owned(),
            });
            events.push(format!("at {:?}", tokens.position(context)));
            token.is_some()
        };
    for step in steps {
        match step {
            | Step::Next => _ = read(&mut tokens, &mut context, &mut events),
            | Step::Save => saved.push(tokens.position(&context)),
            | Step::Restore =>
                if let Some(&position) = saved.last() {
                    tokens.set_position(position);
                },
            | Step::SetLine(line) => tokens.set_line(&context, *line),
        }
    }
    let mut remaining = 0;
    while read(&mut tokens, &mut context, &mut events) {
        remaining += 1;
        assert!(remaining < 10_000, "token sources must reach the end");
    }
    events
}

/// The recorded effects of every case in one test, compared with
/// `tests/fixtures/lexing/<name>.snap`.
struct Snapshot {
    name:   &'static str,
    events: String,
}

impl Snapshot {
    fn new(name: &'static str) -> Self {
        Self {
            name,
            events: String::new(),
        }
    }

    fn record(&mut self, case: &str, events: &[String]) {
        self.events.push_str("=== ");
        self.events.push_str(case);
        self.events.push('\n');
        for event in events {
            self.events.push_str(event);
            self.events.push('\n');
        }
    }

    /// Records walking `source` with `steps` and then to its end.
    fn walk(&mut self, source: &str, steps: &[Step]) {
        let events = walk(source, steps);
        self.record(&format!("{source:?} {steps:?}"), &events);
    }

    /// Records preprocessing `source`, with paths under `directory` spelled
    /// relative to it so the snapshot does not depend on the checkout.
    fn preprocess(&mut self, source: &str, path: &Path, include_directory: Option<&Path>) {
        let mut events = preprocess(source, path, include_directory);
        if let Some(directory) = include_directory {
            let spelled = directory.display().to_string();
            let escaped = spelled.replace('\\', "\\\\");
            for event in &mut events {
                *event = event
                    .replace(&escaped, "test-programs")
                    .replace(&spelled, "test-programs")
                    .replace("test-programs\\\\", "test-programs/")
                    .replace("test-programs\\", "test-programs/");
            }
        }
        let name = path.file_name().map_or_else(
            || path.display().to_string(),
            |name| name.to_string_lossy().into_owned(),
        );
        self.record(&format!("{name} {source:?}"), &events);
    }

    fn finish(self) {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/lexing")
            .join(format!("{}.snap", self.name));
        if std::env::var_os("BLESS").is_some_and(|value| value == "1") {
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, &self.events).unwrap();
            return;
        }
        let expected = std::fs::read_to_string(&path).unwrap_or_else(|error| {
            panic!("{}: {error}; run with BLESS=1 to create it", path.display())
        });
        pretty_assertions::assert_eq!(expected, self.events, "{}", path.display());
    }
}

#[test]
fn ordinary_tokens() {
    let mut snapshot = Snapshot::new("ordinary_tokens");
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
        snapshot.walk(source, &[]);
    }
    snapshot.finish();
}

#[test]
fn comments() {
    let mut snapshot = Snapshot::new("comments");
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
        snapshot.walk(source, &[]);
    }
    snapshot.finish();
}

#[test]
fn phase_one_and_two_rewrites() {
    let mut snapshot = Snapshot::new("phase_one_and_two_rewrites");
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
        snapshot.walk(source, &[]);
    }
    snapshot.finish();
}

#[test]
fn literal_errors_and_missing_newlines() {
    let mut snapshot = Snapshot::new("literal_errors_and_missing_newlines");
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
        snapshot.walk(source, &[]);
    }
    snapshot.finish();
}

#[test]
fn include_operands() {
    let mut snapshot = Snapshot::new("include_operands");
    let steps = [
        Step::Next,
        Step::Next,
        Step::Save,
        Step::Next,
        Step::Restore,
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
        snapshot.walk(source, &steps);
    }
    snapshot.walk("#include <a'b.h>\n", &[]);
    snapshot.finish();
}

#[test]
fn rewinds_and_line_renumbering() {
    let mut snapshot = Snapshot::new("rewinds_and_line_renumbering");
    snapshot.walk(
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
    snapshot.walk(
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
    // supply it again.
    snapshot.walk("", &[Step::Next, Step::SetLine(1)]);
    snapshot.walk("0<>", &[Step::Next, Step::Next, Step::SetLine(1)]);
    snapshot.finish();
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
        1 => Just(Step::Save),
        1 => Just(Step::Restore),
        1 => (1u32..50).prop_map(Step::SetLine),
    ]
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(3000))]

    #[test]
    fn random_sources_lex_to_their_end(
        fragments in proptest::collection::vec(fragment(), 0..40),
        tail in prop::sample::select(vec!["", "\n", "\\\n", "??/\n", "\\\r\n"]),
        steps in proptest::collection::vec(step(), 0..12),
    ) {
        // Rewinds must land on entry boundaries (a debug assertion), and the
        // walk must reach the end of input.
        let source = fragments.concat() + tail;
        let events = walk(&source, &steps);
        prop_assert!(events.last().is_some_and(|event| event.starts_with("at ")));
    }
}

/// Runs phases 1 through 6 and records every parser-facing token and
/// diagnostic.
fn preprocess(source: &str, path: &Path, include_directory: Option<&Path>) -> Vec<String> {
    let tu = crate::util::bump::Bump::new();
    let mut context = Context::new(&tu);
    let directories: SharedVec<PathBuf> = include_directory
        .map(|directory| vec![directory.to_owned()])
        .unwrap_or_default()
        .into();
    let preprocess_arena = crate::util::bump::Bump::new();
    let mut preprocessor = Preprocessor::new(
        &preprocess_arena,
        &mut context,
        path.into(),
        source,
        directories.clone(),
        directories,
    );
    let mut events = Vec::new();
    preprocessor.for_each_item(&mut context, |context, token| {
        drain_diagnostics(context, &mut events);
        events.push(format!(
            "{} {:?}",
            describe_token(token, context, &crate::util::bump::Bump::new()),
            context.get_source_vectors(token.source_vectors)
        ));
    });
    drain_diagnostics(&mut context, &mut events);
    events
}

#[test]
fn test_programs() {
    let directory = Path::new(env!("CARGO_MANIFEST_DIR")).join("test-programs");
    let mut paths: Vec<PathBuf> = std::fs::read_dir(&directory)
        .expect("test-programs is readable")
        .map(|entry| entry.expect("directory entries are readable").path())
        .filter(|path| path.extension().is_some_and(|extension| extension == "c"))
        .collect();
    paths.sort();
    let mut snapshot = Snapshot::new("test_programs");
    let mut checked = 0;
    for path in paths {
        let bytes = std::fs::read(&path).expect("test program is readable");
        let source = String::from_utf8_lossy(&bytes);
        snapshot.preprocess(&source, &path, Some(&directory));
        checked += 1;
    }
    snapshot.finish();
    assert!(checked > 10, "the test-program corpus must be found");
}

#[test]
fn directives_and_macros() {
    let mut snapshot = Snapshot::new("directives_and_macros");
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
        snapshot.preprocess(source, Path::new("<test>"), None);
    }
    snapshot.finish();
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
    fn random_programs_preprocess_to_their_end(
        lines in proptest::collection::vec(program_line(), 0..16),
        crlf in any::<bool>(),
        final_newline in any::<bool>(),
    ) {
        let mut source = lines.join(if crlf { "\r\n" } else { "\n" });
        if final_newline {
            source.push('\n');
        }
        let events = preprocess(&source, Path::new("<test>"), None);
        prop_assert!(!events.iter().any(|event| event.contains("panicked")));
    }
}

#[test]
fn physically_empty_source_has_no_missing_final_newline() {
    let mut snapshot = Snapshot::new("physically_empty_source_has_no_missing_final_newline");
    let tu = crate::util::bump::Bump::new();
    let mut context = Context::new(&tu);
    let file = context.intern_source_file(Path::new("<test>"));
    let source = SharedString::from(String::new());
    context.record_source_text(file, &source);
    let pp = crate::util::bump::Bump::new();
    let mut tokens = TokenSource::new(&mut context, &pp, file, &source);
    assert!(tokens.next_item(&mut context).is_none(), "");
    assert!(context.take_pending_errors().is_empty(), "");
    snapshot.walk("", &[]);
    snapshot.finish();
}

#[test]
fn source_emptied_by_splicing_reports_escaped_final_newline() {
    let mut snapshot = Snapshot::new("source_emptied_by_splicing_reports_escaped_final_newline");
    for source in ["\\\n", "??/\n", "\\\r\n", "\\\n\\\n"] {
        let events = walk(source, &[]);
        assert!(
            events
                .iter()
                .any(|event| event.contains("final newline is escaped")),
            "{source:?}, {events:#?}",
        );
        snapshot.walk(source, &[]);
    }
    snapshot.finish();
}

#[test]
fn unterminated_block_comments_report_the_actual_opener() {
    let mut snapshot = Snapshot::new("unterminated_block_comments_report_the_actual_opener");
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
        let tu = crate::util::bump::Bump::new();
        let mut context = Context::new(&tu);
        let file = context.intern_source_file(Path::new("<test>"));
        let text = SharedString::from(source.to_owned());
        context.record_source_text(file, &text);
        let pp = crate::util::bump::Bump::new();
        let mut tokens = TokenSource::new(&mut context, &pp, file, &text);
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
        assert_eq!(comments.len(), 1, "{source:?}, {errors:#?}");
        let start = SourcePosition {
            index,
            line,
            column,
        };
        assert_eq!(
            comments[0].source_vector,
            SourceVector::new(start, file, source.len() - index),
            "{source:?}",
        );
        if source.starts_with("int") {
            assert!(spellings.iter().any(|spelling| spelling == "sentinel"));
        }
        snapshot.walk(source, &[]);
    }
    snapshot.finish();
}

#[test]
fn terminal_spliced_newlines_report_the_actual_last_splice() {
    let mut snapshot = Snapshot::new("terminal_spliced_newlines_report_the_actual_last_splice");
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
        let tu = crate::util::bump::Bump::new();
        let mut context = Context::new(&tu);
        let file = context.intern_source_file(Path::new("<test>"));
        let text = SharedString::from(source.to_owned());
        context.record_source_text(file, &text);
        let pp = crate::util::bump::Bump::new();
        let mut tokens = TokenSource::new(&mut context, &pp, file, &text);
        while tokens.next_item(&mut context).is_some() {}
        let errors = context.take_pending_errors();
        assert_eq!(errors.len(), 1, "{source:?}: {errors:#?}");
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
        snapshot.walk(source, &[]);
        snapshot.walk(source, &[Step::Save, Step::Next, Step::Restore, Step::Next]);
        snapshot.walk(source, &[Step::Next, Step::Save, Step::Next, Step::Restore]);
        snapshot.walk(
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
    snapshot.finish();
}

#[test]
fn terminal_splice_warning_is_deferred_until_the_tail_is_read() {
    // '\n' is already a complete token; batch construction must stay silent.
    let tu = crate::util::bump::Bump::new();
    let mut context = Context::new(&tu);
    let file = context.intern_source_file(Path::new("<test>"));
    let text = SharedString::from("\n\\\n".to_owned());
    context.record_source_text(file, &text);
    let pp = crate::util::bump::Bump::new();
    let mut tokens = TokenSource::new(&mut context, &pp, file, &text);
    assert!(context.take_pending_errors().is_empty());
    let first = tokens.next_item(&mut context).unwrap();
    assert_eq!(context.string_cache.at(first.contents), "\n");
    assert!(context.take_pending_errors().is_empty());
    // The prior logical LF stays one token; EOF warns without adding one.
    assert!(tokens.next_item(&mut context).is_none());
    assert_eq!(context.take_pending_errors().len(), 1);
}

#[test]
fn cloned_terminal_splice_cursors_keep_independent_warning_state() {
    for source in ["\n\\\n", "word\\\n"] {
        let tu = crate::util::bump::Bump::new();
        let mut context = Context::new(&tu);
        let file = context.intern_source_file(Path::new("<test>"));
        let text = SharedString::from(source.to_owned());
        context.record_source_text(file, &text);
        let pp = crate::util::bump::Bump::new();
        let mut original = TokenSource::new(&mut context, &pp, file, &text);
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

#[test]
fn persisted_sources_resume_where_their_cursor_stood() {
    let spellings = |source: &mut TokenSource<'_>, context: &mut Context<'_>| {
        let mut spellings = Vec::new();
        while let Some(token) = source.next_item(context) {
            spellings.push(context.string_cache.at(token.contents).to_owned());
        }
        spellings
    };
    let tu = crate::util::bump::Bump::new();
    let mut context = Context::new(&tu);
    let file = context.intern_source_file(Path::new("<test>"));
    let text = "a b\nc\n";
    context.record_source_text(file, text);
    let pp = crate::util::bump::Bump::new();
    let mut files = LexedFiles::new_in(&pp);

    // A registered file is shared rather than copied.
    let mut opened = files.open(&mut context, file, text);
    assert!(opened.next_item(&mut context).is_some());
    assert_eq!(files.persist(&opened), opened);

    // A file lexed elsewhere is copied with its reading state.
    let scratch = crate::util::bump::Bump::new();
    let mut temporary = TokenSource::new(&mut context, &scratch, file, text);
    assert!(temporary.next_item(&mut context).is_some());
    let mut persisted = files.persist(&temporary);
    assert_ne!(persisted, temporary);
    let expected = spellings(&mut temporary, &mut context);
    drop(scratch);
    assert_eq!(spellings(&mut persisted, &mut context), expected);
    assert_eq!(expected, [" ", "b", "\n", "c", "\n"]);
}

#[test]
fn lexed_files_grow_in_place_and_keep_exact_storage() {
    let tu = crate::util::bump::Bump::new();
    let mut context = Context::new(&tu);
    let file = context.intern_source_file(Path::new("<test>"));
    // One entry per byte, three times the up-front estimate.
    let text = "a+b;\n".repeat(4096);
    let pp = crate::util::bump::Bump::new();
    let lexed = super::LexedFile::lex(&mut context, &pp, file, &text);
    assert_eq!(lexed.len(), text.len());
    // Exactly the 16-byte aligned entries: growing left no copies behind,
    // and the unused capacity went back to the arena.
    assert_eq!(pp.used(), 16 * text.len());
    assert!(pp.high_water() < 2 * pp.used());
}

#[test]
fn lexing_commits_the_entries_written_not_a_capacity() {
    use crate::util::vm::MAX_COMMIT_STEP;
    let tu = crate::util::bump::Bump::new();
    let mut context = Context::new(&tu);
    let file = context.intern_source_file(Path::new("<test>"));
    // Over five mebibytes of entries, far past any one commit step.
    let text = "a+b;\n".repeat(64 * 1024);
    let pp = crate::util::bump::Bump::new();
    let lexed = super::LexedFile::lex(&mut context, &pp, file, &text);
    assert_eq!(lexed.len(), text.len());
    assert_eq!(pp.used(), 16 * text.len());
    assert_eq!(pp.high_water(), pp.used());
    // Commit followed the entries as they were written, at most one step
    // ahead; a doubling capacity would have committed up to twice as much.
    assert!(
        pp.committed() <= pp.used() + MAX_COMMIT_STEP,
        "{}",
        pp.committed()
    );
}

#[test]
fn lexical_extension_diagnostics_map_spliced_spellings() {
    use crate::configuration::{
        CStandard,
        CompilerConfiguration,
        ExtensionPolicy,
    };

    // Expected ranges are in the physical source, from the first spelling
    // character through the last, including internal but not adjacent splices.
    for (source, spelling, index, line, column, length) in [
        ("int a\\\n<:2];\n", "digraph", 7, 2, 1, 2),
        ("\\\n<:\n", "digraph", 2, 2, 1, 2),
        ("<\\\n:\n", "digraph", 0, 1, 1, 4),
        (":\\\r\n>\n", "digraph", 0, 1, 1, 5),
        ("<\\\n%\n", "digraph", 0, 1, 1, 4),
        ("%\\\n>\n", "digraph", 0, 1, 1, 4),
        ("%\\\n:\n", "digraph", 0, 1, 1, 4),
        ("%\\\n:%\\\n:\n", "digraph", 0, 1, 1, 8),
        ("<:\\\n;\n", "digraph", 0, 1, 1, 2),
        ("/\\\n/ c\n", "//", 0, 1, 1, 4),
        (" /\\\n/ c\n", "//", 1, 1, 2, 4),
        ("\\\n// c\n", "//", 2, 2, 1, 2),
        (" \\\n// c\n", "//", 3, 2, 1, 2),
        ("/\\\r\n/ c\n", "//", 0, 1, 1, 5),
        (" /\\\r\n/ c\n", "//", 1, 1, 2, 5),
        ("//\\\n c\n", "//", 0, 1, 1, 2),
    ] {
        let tu = crate::util::bump::Bump::new();
        let configuration = CompilerConfiguration::new(CStandard::C89, ExtensionPolicy::Warn)
            .with_gnu_extensions(true);
        let mut context = Context::with_configuration(&tu, configuration);
        let file = context.intern_source_file(Path::new("<test>"));
        context.record_source_text(file, source);
        let pp = crate::util::bump::Bump::new();
        let mut tokens = TokenSource::new(&mut context, &pp, file, source);
        while tokens.next_item(&mut context).is_some() {}
        let errors = context.take_pending_errors();
        assert_eq!(errors.len(), 1, "{source:?}: {errors:#?}");
        let TranslationError::Extension(extension) = &errors[0] else {
            panic!("{source:?}: expected an extension diagnostic");
        };
        assert_eq!(extension.spelling(), spelling, "{source:?}");
        let vectors = errors[0].source_vectors(&mut context);
        assert_eq!(
            context.get_source_vectors(vectors),
            &[SourceVector::new(
                SourcePosition {
                    index,
                    line,
                    column,
                },
                file,
                length,
            )],
            "{source:?}",
        );
    }
}

#[test]
fn language_modes_classify_keywords_after_macro_expansion() {
    use crate::configuration::{
        CStandard,
        CompilerConfiguration,
        ExtensionPolicy,
    };
    let source = "#define K __const__\nK _Bool inline restrict bool __typeof__ __declspec\n";
    let mut snapshot = Snapshot::new("language_modes_classify_keywords_after_macro_expansion");
    for (standard, gnu, msvc) in [
        (CStandard::C89, false, false),
        (CStandard::C99, true, false),
        (CStandard::C23, false, true),
    ] {
        let configuration = CompilerConfiguration::new(standard, ExtensionPolicy::Warn)
            .with_gnu_extensions(gnu)
            .with_msvc_extensions(msvc);
        let tu = crate::util::bump::Bump::new();
        let mut context = Context::with_configuration(&tu, configuration);
        let pp_arena = crate::util::bump::Bump::new();
        let mut pp = Preprocessor::new(
            &pp_arena,
            &mut context,
            PathBuf::from("<test>").into_boxed_path(),
            source,
            SharedVec::default(),
            SharedVec::default(),
        );
        let mut events = Vec::new();
        pp.for_each_iterator_item(&mut context, |context, token| {
            drain_diagnostics(context, &mut events);
            events.push(format!(
                "{:?} {} {:?}",
                token.kind,
                describe_token(token, context, &tu),
                context.get_source_vectors(token.source_vectors)
            ));
        });
        drain_diagnostics(&mut context, &mut events);
        snapshot.record(&format!("{standard:?}, GNU={gnu}, MSVC={msvc}"), &events);
    }
    snapshot.finish();
}
