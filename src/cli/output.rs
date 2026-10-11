use std::fmt::Write as _;

use super::{
    ArenaString,
    Args,
    Bump,
    Context,
    DiagnosticReporter,
    HeaderSearch,
    InspectionOptions,
    ParsedTranslationUnit,
    Path,
    RenderColor,
    Write,
    describe_token,
    io,
    parse_translation_unit,
    preprocess_with_diagnostics,
    with_preprocessor,
};

pub(super) fn print_parser_output<'tu>(
    context: &mut Context<'tu>,
    source_filename: &Path,
    input_string: &'tu str,
    search: HeaderSearch<'_>,
    output: &ParserOutput,
    repeated_specifier_warnings: bool,
) {
    context.configuration = context
        .configuration
        .with_repeated_specifier_warnings(repeated_specifier_warnings);
    let unit = parse_translation_unit(context, source_filename, input_string, search);
    let semantic = (output.semantic_types || (!output.syntax_tree && !output.raw_syntax))
        .then(|| crate::pipeline::analyze_translation_unit(context, &unit));
    let reporter_arena = Bump::new();
    let mut reporter = DiagnosticReporter::new(
        &reporter_arena,
        context.tu_arena(),
        RenderColor::for_stderr(),
    );
    let stderr = &mut io::stderr();
    expect_stderr(reporter.report_pending(context, stderr));

    if output.syntax_tree {
        let inspection = Bump::new();
        eprint!(
            "{}",
            unit.inspect(
                &inspection,
                context,
                InspectionOptions {
                    show_locations: output.syntax_locations,
                },
            )
        );
    }
    if output.raw_syntax {
        print_raw_syntax(&unit);
    }
    if let Some(semantic) = semantic
        && output.semantic_types
    {
        let inspection = Bump::new();
        eprint!("{}", semantic.inspect(context, &inspection));
    }
    expect_stderr(reporter.finish(context, stderr));
}

/// Prints the preprocessed translation unit, each token after the
/// diagnostics its production reported.
pub(super) fn print_preprocessor_output<'tu>(
    context: &mut Context<'tu>,
    source_filename: &Path,
    input_string: &'tu str,
    search: HeaderSearch<'_>,
) {
    let mut reporter_arena = Bump::new();
    let mut stderr = io::stderr();
    print_preprocessor_output_in(
        context,
        source_filename,
        input_string,
        search,
        TokenOutput {
            out:            &mut stderr,
            reporter_arena: &mut reporter_arena,
            color:          RenderColor::for_stderr(),
        },
    );
}

/// Keeps only the diagnostic counts between token batches. Each batch's
/// folding state and copied locations are discarded before the next token.
pub(super) fn print_preprocessor_output_in<'tu>(
    context: &mut Context<'tu>,
    source_filename: &Path,
    input_string: &'tu str,
    search: HeaderSearch<'_>,
    TokenOutput {
        out,
        reporter_arena,
        color,
    }: TokenOutput<'_>,
) {
    let mut scratch = Bump::new();
    let items = with_preprocessor(
        context,
        source_filename,
        input_string,
        search,
        |preprocessor, context, _pp| preprocess_with_diagnostics(preprocessor, context),
    );
    let mut items = items.into_iter();
    let (mut errors, mut warnings) = (0, 0);
    loop {
        let mut reporter = DiagnosticReporter::new(reporter_arena, context.tu_arena(), color);
        reporter.errors = errors;
        reporter.warnings = warnings;
        let token = loop {
            match items.next() {
                | Some(Err(error)) => reporter.report(&error, context),
                | Some(Ok(token)) => {
                    expect_stderr(reporter.flush(context, out));
                    break Some(token);
                },
                | None => {
                    expect_stderr(reporter.finish(context, out));
                    break None;
                },
            }
        };
        errors = reporter.errors;
        warnings = reporter.warnings;
        drop(reporter);
        // The reporter's copied locations and folding state are gone; only
        // counts survive, and no raw pointer into its arena is retained.
        reporter_arena.reset();
        match token {
            | Some(token) => {
                // The previous token description was written synchronously;
                // neither the token nor context borrows this scratch arena.
                scratch.reset();
                expect_stderr(writeln!(
                    out,
                    "{}",
                    describe_token(token, context, &scratch)
                ));
            },
            | None => break,
        }
    }
}

pub(super) struct TokenOutput<'a> {
    pub(super) out:            &'a mut dyn Write,
    pub(super) reporter_arena: &'a mut Bump,
    pub(super) color:          RenderColor,
}

/// Prints the whole syntax tree in Rust debug form to stderr, rendered in
/// an arena on a thread with a stack deep enough for deeply nested syntax.
#[expect(
    clippy::disallowed_methods,
    reason = "std's thread builder takes the name as a `String`; spawning the deep-stack thread \
              allocates in std anyway."
)]
pub(super) fn print_raw_syntax(unit: &ParsedTranslationUnit<'_>) {
    let raw = unit.raw_debug();
    std::thread::scope(|scope| {
        std::thread::Builder::new()
            .name("raw-syntax".to_owned())
            .stack_size(RAW_SYNTAX_STACK_BYTES)
            .spawn_scoped(scope, move || {
                let arena = Bump::new();
                let mut text = ArenaString::new_in(&arena);
                _ = write!(text, "{raw:#?}");
                eprintln!("{text}");
            })
            .expect("the raw syntax thread starts")
            .join()
            .expect("raw syntax rendering finishes");
    });
}

#[derive(Args)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "Clap owns independent inspection flags with explicit conflicts."
)]
pub(super) struct ParserOutput {
    /// Print resolved declarations, types, linkage and storage duration.
    #[clap(long, conflicts_with_all = ["tokens", "syntax_tree", "raw_syntax"])]
    pub(super) semantic_types:   bool,
    /// Print a deterministic, source-oriented C syntax tree.
    #[clap(long, conflicts_with = "tokens")]
    pub(super) syntax_tree:      bool,
    /// Include line and column locations in `--syntax-tree` output.
    #[clap(long, requires = "syntax_tree")]
    pub(super) syntax_locations: bool,
    /// Print the raw syntax tree, in Rust debug form, for storage debugging.
    #[clap(long, conflicts_with = "tokens")]
    pub(super) raw_syntax:       bool,
}

#[derive(Args)]
pub(super) struct CliOutput {
    /// Print preprocessor tokens instead of parser output.
    #[clap(long)]
    pub(super) tokens: bool,
    #[command(flatten)]
    pub(super) parser: ParserOutput,
}

/// Fails like `eprint!` when stderr cannot be written.
pub(super) fn expect_stderr(result: io::Result<()>) {
    if let Err(error) = result {
        panic!("failed printing to stderr: {error}");
    }
}

/// Stack reserved for rendering `--raw-syntax`. The derived `Debug` output
/// recurses once per nesting level of the tree, which the iterative parser
/// accepts far deeper than the main thread's stack allows. The stack is
/// reserved, not committed, so unused depth costs only address space.
pub(super) const RAW_SYNTAX_STACK_BYTES: usize = 1 << 30;
