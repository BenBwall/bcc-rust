//! Every observable effect of phases 1-6 (tokens, provenance, and rendered
//! diagnostics) for edge cases, compared with
//! `tests/fixtures/lexing/preprocessing_edge_cases.snap`. Run with `BLESS=1`
//! to rewrite it. The snapshot was first produced while the batch pipeline
//! still had to agree with the streaming one.

use std::{
    fmt::Write,
    path::PathBuf,
};

use proptest::prelude::*;

use crate::{
    cli::describe_token,
    diagnostics::{
        ColorChoice,
        Renderer,
        ToDiagnostic,
    },
    translation_phases::{
        Context,
        GetSourceVectors,
        preprocessing::{
            Preprocessor,
            Token,
        },
    },
    util::shared::SharedVec,
};

fn token_description(token: Token, context: &Context<'_>) -> String {
    format!(
        "{} {:?}",
        describe_token(token, context, &crate::util::bump::Bump::new()),
        context.get_source_vectors(token.source_vectors),
    )
}

/// The diagnostics, then the tokens, of preprocessing `source`.
fn observe(source: &str) -> Vec<String> {
    let tu = crate::util::bump::Bump::new();
    let mut context = Context::new(&tu);
    let preprocess_arena = crate::util::bump::Bump::new();
    let mut preprocessor = Preprocessor::new(
        &preprocess_arena,
        &mut context,
        PathBuf::from("<test>").into_boxed_path(),
        source,
        SharedVec::default(),
        SharedVec::default(),
    );
    let tokens = preprocessor.preprocess_all(&mut context);
    let mut events = Vec::new();
    while let Some(error) = context.pop_pending_error() {
        let sources = error.source_vectors(&mut context);
        let diagnostic = error.to_diagnostic(&context, sources);
        events.push(format!(
            "{:?}\n{}",
            context.get_source_vectors(sources),
            Renderer::new(ColorChoice::Plain).render(&diagnostic, &context),
        ));
    }
    events.extend(
        tokens
            .into_iter()
            .map(|token| token_description(token, &context)),
    );
    events
}

#[test]
fn edge_cases_preserve_tokens_diagnostics_and_provenance() {
    let mut snapshot = String::new();
    for source in [
        "",
        "int x = 1;\n",
        "#define F() yes\nF()\n",
        "#define F(x) x\nF\n(yes)\n",
        "#define S(x) #x\nS(  a  b  )\n",
        "#define CAT(a,b) a ## b\nCAT(<,:) CAT(-,>)\n",
        "#define A F\n#define F(x) x\nA(yes)\n",
        "#line 10 \"logical.c\"\n__FILE__ __LINE__\n",
        "#if (1)\nyes\n#endif\n",
        "#if (1 ? -1 : 0U) > 0\nyes\n#endif\n",
        "#if 0 && 1 / 0\nno\n#else\nyes\n#endif\n",
        "#pragma ignored tokens\nafter\n",
        "\"a\\q\" \"b\" 0xg identifier\n",
        "#define C(a,b) a ## b\n#define M(x) C(x,0) C(x,1)\nM(int value)\n",
        "#define J(a,b,c) a##b##c\nJ(x,_,y) J(m n,o,p q) J(,,z) J(.,.,.)\n",
        "int sentinel; /* unterminated\n",
        "#error text here\nafter\n",
        "??=define X 1\r\nX ??/\r\n+ 2\r\n",
        "#if 0\n#else\nyes\n#else\nwrong\n#endif\nafter\n",
        "#if 0\n#else extra\nyes\n#endif\nafter\n",
        "#\n#define A 1\n#undef A\n#ifdef A\nwrong\n#else\nyes\n#endif\nafter\n",
    ] {
        _ = writeln!(snapshot, "=== {source:?}");
        for event in observe(source) {
            snapshot.push_str(&event);
            snapshot.push('\n');
        }
    }
    crate::test_support::assert_snapshot(
        "tests/fixtures/lexing/preprocessing_edge_cases.snap",
        &snapshot,
    );
}

#[test]
fn line_directive_inside_macro_argument_does_not_rewind_another_source() {
    drop(observe("#define F(x) [x]\nF(\n#line 10\n)"));
}

fn program_line() -> impl Strategy<Value = &'static str> {
    prop::sample::select(vec![
        "#define A 1",
        "#define A 2",
        "#undef A",
        "#define F(x) [x]",
        "#define S(x) #x",
        "#define C(a,b) a ## b",
        "F(a)",
        "F\n(a)",
        "S( a  b  )",
        "C(a,b)",
        "A + 2",
        "#if A",
        "#if (1)",
        "#elif 0",
        "#else",
        "#endif",
        "#pragma ignored trailing",
        "#line 10",
        "#line 4 \"logical.c\"",
        "__FILE__ __LINE__",
        "\"one\" \"two\" identifier",
        "\"\\q\" 0xg",
        "/* comment */ token",
        "\"unterminated",
        "#error message",
        "#include",
        "#include <missing.h",
        "#include \"missing\\",
        "F(",
        ")",
        "a\\",
    ])
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(500))]

    /// Generated programs preprocess to their end with renderable
    /// diagnostics.
    #[test]
    fn generated_programs_preprocess_to_their_end(
        lines in proptest::collection::vec(program_line(), 0..16),
        crlf in any::<bool>(),
        final_newline in any::<bool>(),
    ) {
        let mut source = lines.join(if crlf { "\r\n" } else { "\n" });
        if final_newline { source.push('\n'); }
        drop(observe(&source));
    }
}

#[test]
fn unfinished_quoted_include_in_macro_argument_preserves_rewind_boundary() {
    let source = "#define F(x) [x]\nF(\n#include \"missing\\\n#define A 1\n)";
    drop(observe(source));
}
