//! Entry points for the Criterion and coz benchmarks.

use std::{
    fmt::Write,
    path::Path,
    sync::OnceLock,
};

use crate::{
    translation_phases::{
        Context,
        TranslationPhase,
        box_path_from_str,
        parsing::Parser,
        preprocessing::Preprocessor,
        preprocessor_tokenizer::TokenSource,
    },
    util::shared::SharedVec,
};

#[doc(hidden)]
#[must_use]
pub fn preprocess_one_million() -> usize {
    preprocess(BenchmarkInput::OneMillionLines)
}

/// A generated benchmark translation unit.
#[doc(hidden)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BenchmarkInput {
    /// One million `int iN = N;` lines.
    OneMillionLines,
    /// Typedef-heavy declarations, aggregates, and function bodies.
    ParserMix,
    /// Function-like macros, token pasting, stringification, and
    /// conditional groups expanding into ordinary C.
    MacroMix,
    /// Function bodies of deeply nested expressions: every precedence level,
    /// casts, calls, subscripts, member access, and compound literals.
    ExpressionHeavy,
    /// File-scope declarations with nested declarators, prototypes, K&R
    /// definitions, aggregates, bit-fields, and designated initializers.
    DeclarationHeavy,
    /// Isolated string literals, stressing phase-6 lookahead without
    /// concatenation.
    SingleStrings,
    /// Empty function-like macro invocations.
    EmptyMacros,
    /// Reused parameters and nested argument prescans.
    NestedArguments,
}

impl BenchmarkInput {
    /// Inputs measured for every translation phase.
    pub const ALL: [Self; 3] = [Self::OneMillionLines, Self::ParserMix, Self::MacroMix];
    /// Parser production stress inputs.
    pub const PARSER_STRESS: [Self; 2] = [Self::ExpressionHeavy, Self::DeclarationHeavy];
    /// Extra inputs that isolate preprocessing allocation costs.
    pub const PREPROCESSOR_STRESS: [Self; 3] = [
        Self::SingleStrings,
        Self::EmptyMacros,
        Self::NestedArguments,
    ];

    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            | Self::OneMillionLines => "one million lines",
            | Self::ParserMix => "mixed C99 workload",
            | Self::MacroMix => "macro-heavy workload",
            | Self::ExpressionHeavy => "expression-heavy workload",
            | Self::DeclarationHeavy => "declaration-heavy workload",
            | Self::SingleStrings => "single string literals",
            | Self::EmptyMacros => "empty macro calls",
            | Self::NestedArguments => "nested reused arguments",
        }
    }

    /// # Panics
    ///
    /// Never for the generated inputs, whose sizes fit in `u64`.
    #[must_use]
    pub fn bytes(self) -> u64 {
        u64::try_from(self.source().len()).expect("benchmark input length must fit in u64")
    }

    /// # Panics
    ///
    /// Never for the generated inputs, whose line counts fit in `u64`.
    #[must_use]
    pub fn lines(self) -> u64 {
        u64::try_from(self.source().lines().count()).expect("benchmark line count must fit in u64")
    }

    #[expect(
        clippy::large_include_file,
        reason = "The generated benchmark inputs are intentionally large."
    )]
    fn source(self) -> &'static str {
        match self {
            | Self::OneMillionLines =>
                include_str!(concat!(env!("OUT_DIR"), "/one-million-lines.c")),
            | Self::ParserMix => include_str!(concat!(env!("OUT_DIR"), "/parser-mix.c")),
            | Self::MacroMix => include_str!(concat!(env!("OUT_DIR"), "/macro-mix.c")),
            | Self::SingleStrings => {
                static SOURCE: OnceLock<String> = OnceLock::new();
                SOURCE.get_or_init(|| "\"text\";\n".repeat(20_000))
            },
            | Self::EmptyMacros => {
                static SOURCE: OnceLock<String> = OnceLock::new();
                SOURCE.get_or_init(|| format!("#define EMPTY()\n{}", "EMPTY() ;\n".repeat(20_000)))
            },
            | Self::NestedArguments => {
                static SOURCE: OnceLock<String> = OnceLock::new();
                SOURCE.get_or_init(|| {
                    format!(
                        "#define ID(x) x\n#define TWICE(x) x + x\n{}",
                        "TWICE(ID(7));\n".repeat(20_000)
                    )
                })
            },
            | Self::ExpressionHeavy => {
                static SOURCE: OnceLock<String> = OnceLock::new();
                SOURCE.get_or_init(|| expression_heavy_source(4_000))
            },
            | Self::DeclarationHeavy => {
                static SOURCE: OnceLock<String> = OnceLock::new();
                SOURCE.get_or_init(|| declaration_heavy_source(6_000))
            },
        }
    }
}

/// `count` functions whose statements nest expressions of every precedence
/// level.
fn expression_heavy_source(count: usize) -> String {
    let mut source = String::from(
        "typedef struct point { int x, y; } point;
         int table[64];
         int combine(int a, int b, int c) { return a + b * c; }
",
    );
    for index in 0..count {
        let _ = write!(
            source,
            "int expressions{index}(int a, int b, int c, point *p)
             {{
                 int r = (a + b * c - (a << 2) / (b | 1)) % 7 ^ ~c & (a >= b || b != c);
                 r += a ? b ? c : -a : (int)sizeof(point) + (int)sizeof r;
                 r = combine(table[(a + {index}) & 63], p->x * p[0].y, combine(a, b, c))              << (r > 3 && r < 9);
                 r = ((point){{ .x = a, .y = b }}).x + (r *= 2, r -= c, r);
                 p->y = !r ? -(a - (b - (c - (a - (b - c))))) : ++r + r-- * (unsigned char)a;
                 return (((r + a) * (b + c)) / ((a - b) | 1)) + table[r & 63];
             }}
"
        );
    }
    source
}

/// `count` groups of file-scope declarations with nested declarators,
/// aggregates, and initializers.
fn declaration_heavy_source(count: usize) -> String {
    let mut source = String::new();
    for index in 0..count {
        let _ = write!(
            source,
            "typedef unsigned long size{index};
             typedef int (*handler{index})(int, char *restrict, ...);
             struct node{index} {{ size{index} length : 12; unsigned flags : 4;              struct node{index} *next, *(*links[2])(void); union {{ int i; float f; }} u; }};
             enum state{index} {{ IDLE{index}, BUSY{index} = 4, DONE{index}, }};
             static const int (*const lookup{index}[4])(size{index} *, const char *) = {{ 0 }};
             extern void (*signal{index}(int, void (*)(int)))(int);
             struct node{index} root{index} = {{ .length = {index} % 4096, .flags = 3,              .next = 0, .u = {{ .f = 1.5f }} }};
             int matrix{index}[2][3] = {{ [1] = {{ 1, 2 }}, [0][2] = 7 }};
             char *names{index}[] = {{ \"a\", \"b\" \"c\", 0 }};
             int old_style{index}(a, b, c) int a; char *b; double c; {{ return a; }}
             handler{index} callbacks{index}[3], (*selected{index})(int, char *restrict, ...);
"
        );
    }
    source
}

/// Runs translation phases 1 through 3 only and returns the number of
/// preprocessing tokens.
#[doc(hidden)]
#[must_use]
pub fn lex(input: BenchmarkInput) -> usize {
    let tu = crate::util::bump::Bump::new();
    let mut context = Context::new(&tu);
    let file = context.intern_source_file(Path::new("<input>"));
    let mut tokens = TokenSource::new(&mut context, file, input.source());
    let mut count = 0;
    while tokens.next_item(&mut context).is_some() {
        count += 1;
        // Lexing alone never compacts provenance, so keep the arena from
        // dominating the measurement.
        context.source_vectors.0.clear();
    }
    count
}

/// Opens `input` as the main source file.
fn preprocessor(context: &mut Context<'_>, input: BenchmarkInput) -> Preprocessor {
    Preprocessor::new(
        context,
        box_path_from_str("<input>"),
        input.source(),
        SharedVec::default(),
        SharedVec::default(),
    )
}

/// Runs translation phases 1 through 6 over the whole translation unit and
/// returns the number of parser-facing tokens. Their provenance is kept, as
/// the parser reading them would need.
#[doc(hidden)]
#[must_use]
pub fn preprocess(input: BenchmarkInput) -> usize {
    let tu = crate::util::bump::Bump::new();
    let mut context = Context::new(&tu);
    preprocessor(&mut context, input)
        .preprocess_all(&mut context)
        .len()
}

/// Summary of one benchmarked parse, returned so the work cannot be elided
/// and so benchmark setup can reject inputs that produce diagnostics.
#[doc(hidden)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ParseBenchmarkSummary {
    pub external_declarations: usize,
    pub diagnostics:           usize,
}

/// Runs translation phases 1 through 7 and summarizes the parse.
#[doc(hidden)]
#[must_use]
pub fn parse(input: BenchmarkInput) -> ParseBenchmarkSummary {
    let tu = crate::util::bump::Bump::new();
    let mut context = Context::new(&tu);
    prepare_parse_in_context(&mut context, input).parse()
}

/// A translation unit preprocessed through phase 6 and ready to parse, so a
/// benchmark can time phase 7 alone.
#[doc(hidden)]
pub struct PreparedParse<'a, 'tu> {
    context: &'a mut Context<'tu>,
    parser:  Parser,
}

impl std::fmt::Debug for PreparedParse<'_, '_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PreparedParse").finish_non_exhaustive()
    }
}

/// Runs translation phases 1 through 6, then passes a prepared parser to a
/// callback while its translation context remains alive.
#[doc(hidden)]
pub fn with_prepared_parse<R>(
    input: BenchmarkInput,
    inspect: impl FnOnce(PreparedParse<'_, '_>) -> R,
) -> R {
    let tu = crate::util::bump::Bump::new();
    let mut context = Context::new(&tu);
    inspect(prepare_parse_in_context(&mut context, input))
}

fn prepare_parse_in_context<'a, 'tu>(
    context: &'a mut Context<'tu>,
    input: BenchmarkInput,
) -> PreparedParse<'a, 'tu> {
    let preprocessor = preprocessor(context, input);
    let parser = Parser::new(preprocessor, context);
    PreparedParse { context, parser }
}

impl PreparedParse<'_, '_> {
    /// Runs translation phase 7 and summarizes the parse.
    #[must_use]
    pub fn parse(self) -> ParseBenchmarkSummary {
        let unit = self.parser.parse_translation_unit(self.context);
        ParseBenchmarkSummary {
            external_declarations: unit.external_declarations().len(),
            diagnostics:           self.context.take_pending_errors().len(),
        }
    }
}
