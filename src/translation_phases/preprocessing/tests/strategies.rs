//! Observable phase 1-6 behavior across every preprocessing strategy.

use std::path::PathBuf;

use proptest::prelude::*;

use crate::{
    cli::describe_token,
    diagnostics::{
        ColorChoice,
        Renderer,
        ToDiagnostic,
    },
    pipeline::PreprocessingStrategy,
    translation_phases::{
        Context,
        GetSourceVectors,
        preprocessing::Token,
    },
    util::shared::SharedVec,
};

#[derive(Debug, PartialEq, Eq)]
struct Snapshot {
    tokens:      Vec<String>,
    diagnostics: Vec<String>,
}

fn token_description(token: Token, context: &Context) -> String {
    format!(
        "{} {:?}",
        describe_token(token, context),
        context.get_source_vectors(token.source_vectors),
    )
}

fn drain_diagnostics(context: &mut Context, snapshot: &mut Snapshot) {
    while let Some(error) = context.pop_pending_error() {
        let sources = error.source_vectors(context);
        let diagnostic = error.to_diagnostic(context, sources);
        snapshot.diagnostics.push(format!(
            "{:?}\n{}",
            context.get_source_vectors(sources),
            Renderer::new(ColorChoice::Plain).render(&diagnostic, context),
        ));
    }
}

fn snapshot(source: &str, strategy: PreprocessingStrategy) -> Snapshot {
    let mut context = Context::new();
    let mut preprocessor = strategy.preprocessor(
        &mut context,
        PathBuf::from("<test>").into_boxed_path(),
        source.to_owned().into(),
        SharedVec::default(),
        SharedVec::default(),
    );
    let mut snapshot = Snapshot {
        tokens:      Vec::new(),
        diagnostics: Vec::new(),
    };
    if strategy == PreprocessingStrategy::Batch {
        let tokens = preprocessor.preprocess_all(&mut context);
        drain_diagnostics(&mut context, &mut snapshot);
        for token in tokens {
            snapshot.tokens.push(token_description(token, &context));
        }
    } else {
        loop {
            let token = preprocessor.next_iterator_item(&mut context);
            drain_diagnostics(&mut context, &mut snapshot);
            let Some(token) = token else { break };
            snapshot.tokens.push(token_description(token, &context));
        }
    }
    snapshot
}

pub(super) fn assert_strategies_agree(source: &str) {
    let streaming = snapshot(source, PreprocessingStrategy::Streaming);
    for strategy in [
        PreprocessingStrategy::BatchLexing,
        PreprocessingStrategy::Batch,
    ] {
        let actual = snapshot(source, strategy);
        pretty_assertions::assert_eq!(streaming, actual, "{strategy:?}: {source:?}");
    }
}

#[test]
fn edge_cases_preserve_tokens_diagnostics_and_provenance_across_strategies() {
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
        "int sentinel; /* unterminated\n",
        "#error text here\nafter\n",
        "??=define X 1\r\nX ??/\r\n+ 2\r\n",
    ] {
        assert_strategies_agree(source);
    }
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
        "F(",
        ")",
    ])
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(500))]

    #[test]
    fn generated_programs_preserve_strategy_observables(
        lines in proptest::collection::vec(program_line(), 0..16),
        crlf in any::<bool>(),
        final_newline in any::<bool>(),
    ) {
        let mut source = lines.join(if crlf { "\r\n" } else { "\n" });
        if final_newline { source.push('\n'); }
        assert_strategies_agree(&source);
    }
}
