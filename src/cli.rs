//! Command-line interface: argument parsing, token and syntax-tree output,
//! and diagnostic reporting.

use std::{
    env::{
        split_paths,
        var_os,
    },
    path::{
        Path,
        PathBuf,
    },
};

use clap::{
    Args,
    ColorChoice,
    Parser,
};
use thiserror::Error;

use crate::{
    diagnostics::{
        ColorChoice as RenderColor,
        Diagnostic,
        Renderer,
        ToDiagnostic,
        count_of,
    },
    pipeline::preprocess_with_diagnostics,
    translation_phases::{
        Context,
        ErrorSeverity,
        GetSourceVectors,
        SourceVector,
        TranslationError,
        parsing::{
            InspectionOptions,
            Parser as LanguageParser,
        },
        preprocessing::{
            CharacterTokenType,
            IntegerTokenType,
            Preprocessor,
            StringTokenType,
            Token,
            TokenType,
        },
    },
    util::HashMap,
};

#[derive(Parser)]
#[command(author, version, about, long_about, color = ColorChoice::Always)]
struct Cli {
    #[command(flatten)]
    input: CliInput,
    /// Add directory to include search path.
    #[clap(short = 'q', long = "iquote")]
    quote_include: Vec<PathBuf>,
    /// Add directory to system include search path.
    #[clap(short = 's', long = "isystem")]
    system_include: Vec<PathBuf>,
    #[command(flatten)]
    output: CliOutput,
    /// Suppress the `repeated-specifiers` quality warning group.
    #[clap(long)]
    no_repeated_specifier_warnings: bool,
}

#[derive(Args)]
struct CliOutput {
    /// Print preprocessor tokens instead of parser output.
    #[clap(long)]
    tokens: bool,
    #[command(flatten)]
    parser: ParserOutput,
}

#[derive(Args)]
struct ParserOutput {
    /// Print a deterministic, source-oriented C syntax tree.
    #[clap(long, conflicts_with = "tokens")]
    syntax_tree:      bool,
    /// Include line and column locations in `--syntax-tree` output.
    #[clap(long, requires = "syntax_tree")]
    syntax_locations: bool,
    /// Print raw parser arenas for storage debugging.
    #[clap(long, conflicts_with = "tokens")]
    raw_syntax:       bool,
}

#[derive(Args)]
#[group(required = true, multiple = false)]
struct CliInput {
    /// Input string to be parsed.
    #[clap(short, long, conflicts_with = "input_file")]
    input:      Option<String>,
    /// Input file to be parsed.
    #[clap(conflicts_with = "input")]
    input_file: Option<PathBuf>,
}

#[doc(hidden)]
#[derive(Debug, Error)]
pub enum MainError {
    #[error("error: cannot read `{}`: {source}", path.display())]
    OpenInputFileError {
        path:   PathBuf,
        source: std::io::Error,
    },
    #[error(transparent)]
    ParseArgumentsError(#[from] clap::Error),
}

/// Returns the directories of a GCC-style search-path variable.
///
/// Elements use the platform separator (`;` on Windows, `:` elsewhere). As
/// in GCC and Clang, an empty element names the working directory, while an
/// unset or empty variable contributes nothing.
fn include_path_from_env(env_var: &str) -> Vec<PathBuf> {
    match var_os(env_var) {
        | Some(value) if !value.is_empty() => split_paths(&value)
            .map(|path| {
                if path.as_os_str().is_empty() {
                    PathBuf::from(".")
                } else {
                    path
                }
            })
            .collect(),
        | _ => Vec::new(),
    }
}

#[doc(hidden)]
pub fn run() -> Result<(), MainError> {
    let mut args = Cli::try_parse()?;
    let tu = crate::util::bump::Bump::new();
    let input_argument = args.input.input.take();
    let (input_string, source_filename): (&str, PathBuf) =
        match (input_argument.as_deref(), args.input.input_file.take()) {
            | (Some(input), None) => (tu.alloc_str(input), PathBuf::from("<input>")),
            | (None, Some(input_file)) => (
                tu.read_to_str_lossy(&input_file).map_err(|source| {
                    MainError::OpenInputFileError {
                        path: input_file.clone(),
                        source,
                    }
                })?,
                input_file,
            ),
            | _ => unreachable!("clap requires exactly one input source"),
        };
    // GCC searches `CPATH` like `-I` (before `-isystem`) and
    // `C_INCLUDE_PATH` like a trailing `-isystem`.
    let mut system_include = include_path_from_env("CPATH");
    system_include.append(&mut args.system_include);
    system_include.extend(include_path_from_env("C_INCLUDE_PATH"));
    args.system_include = system_include;
    let mut context = Context::new(&tu);

    if args.output.tokens {
        print_preprocessor_output(
            &mut context,
            &source_filename,
            input_string,
            &args.quote_include,
            &args.system_include,
        );
    } else {
        print_parser_output(
            &mut context,
            &source_filename,
            input_string,
            &args.quote_include,
            &args.system_include,
            &args.output.parser,
            !args.no_repeated_specifier_warnings,
        );
    }
    Ok(())
}

/// Prints the preprocessed translation unit, each token after the
/// diagnostics its production reported.
fn print_preprocessor_output<'tu>(
    context: &mut Context<'tu>,
    source_filename: &Path,
    input_string: &'tu str,
    quote_include: &[PathBuf],
    system_include: &[PathBuf],
) {
    let preprocessor = Preprocessor::new_with_arena_source(
        context,
        source_filename,
        input_string,
        quote_include,
        system_include,
    );
    let mut reporter = DiagnosticReporter::new();
    for item in preprocess_with_diagnostics(preprocessor, context) {
        match item {
            | Ok(token) => {
                reporter.flush(context);
                eprintln!("{}", describe_token(token, context));
            },
            | Err(error) => reporter.report(&error, context),
        }
    }
    reporter.finish(context);
}

/// One line per token: its location, kind, source spelling, and for
/// constants the value and type the preprocessor assigned.
pub(crate) fn describe_token(token: Token, context: &Context<'_>) -> String {
    let spelling = context
        .string_cache
        .at(token.contents)
        .trim_end_matches('\0');
    let description = match token.kind {
        | TokenType::Identifier => format!("identifier `{spelling}`"),
        | TokenType::Keyword(keyword) => format!("keyword `{}`", keyword.spelling()),
        | TokenType::Operator(operator) => format!("punctuator `{}`", operator.spelling()),
        | TokenType::String(StringTokenType::String(contents)) => format!(
            "string literal {}",
            context.literal_spelling(contents, false)
        ),
        | TokenType::String(StringTokenType::WideString(contents)) => format!(
            "wide string literal {}",
            context.literal_spelling(contents, true)
        ),
        | TokenType::Character(character) => {
            let (value, type_name) = match character {
                | CharacterTokenType::Char(c) => (i64::from(u32::from(c)), "int"),
                | CharacterTokenType::WideChar(c) => (i64::from(c), "wchar_t"),
                | CharacterTokenType::MultiChar(value) => (i64::from(value), "int"),
            };
            format!("character constant `{spelling}` = {value} ({type_name})")
        },
        | TokenType::Integer(integer) => {
            let (value, type_name) = match integer {
                | IntegerTokenType::Int(value) => (i128::from(value), "int"),
                | IntegerTokenType::Long(value) => (i128::from(value.get()), "long"),
                | IntegerTokenType::LongLong(value) => (i128::from(value.get()), "long long"),
                | IntegerTokenType::UnsignedInt(value) => (i128::from(value), "unsigned int"),
                | IntegerTokenType::UnsignedLong(value) =>
                    (i128::from(value.get()), "unsigned long"),
                | IntegerTokenType::UnsignedLongLong(value) =>
                    (i128::from(value.get()), "unsigned long long"),
            };
            format!("integer constant `{spelling}` = {value} ({type_name})")
        },
        | TokenType::Float(float) => format!(
            "floating constant `{spelling}` = {float} ({})",
            float.type_name()
        ),
    };
    match context.get_source_vectors(token.source_vectors).first() {
        | Some(vector) => format!(
            "{}:{}:{}: {description}",
            context.get_source_file(vector.source_file_index).display(),
            vector.line,
            vector.column,
        ),
        | None => description,
    }
}

fn print_parser_output<'tu>(
    context: &mut Context<'tu>,
    source_filename: &Path,
    input_string: &'tu str,
    quote_include: &[PathBuf],
    system_include: &[PathBuf],
    output: &ParserOutput,
    repeated_specifier_warnings: bool,
) {
    context.configuration = context
        .configuration
        .with_repeated_specifier_warnings(repeated_specifier_warnings);
    let preprocessor = Preprocessor::new_with_arena_source(
        context,
        source_filename,
        input_string,
        quote_include,
        system_include,
    );
    let preprocessed = LanguageParser::preprocess(preprocessor, context);
    let unit = LanguageParser::from_preprocessed(preprocessed).parse_translation_unit(context);
    let mut reporter = DiagnosticReporter::new();
    while let Some(error) = context.pop_pending_error() {
        reporter.report(&error, context);
    }
    reporter.order_source_runs();
    reporter.flush(context);

    if output.syntax_tree {
        eprint!(
            "{}",
            unit.syntax().inspect(
                unit.external_declarations(),
                context,
                InspectionOptions {
                    show_locations: output.syntax_locations,
                },
            )
        );
    }
    if output.raw_syntax {
        let rendered = format!("{:#?}", unit.syntax().raw_debug());
        eprintln!("{rendered}");
    }
    reporter.finish(context);
}

/// Renders diagnostics to stderr and summarizes them at the end, like
/// `N errors and M warnings generated`.
///
/// An error reported at exactly the same place as an earlier error from the
/// same mistake is folded into it rather than printed again: a parser error
/// into the parser error just before it when no input was consumed between
/// them, otherwise into a lexing or preprocessing error at that place; a
/// preprocessing error into the preprocessing error just before it. The
/// parser and the preprocessor are considered separately.
struct DiagnosticReporter {
    renderer:     Renderer,
    pending:      Vec<PendingDiagnostic>,
    /// The parser diagnostic reported last: where it was folded or stored,
    /// and the input the parser had consumed by then.
    last_parser:  Option<(usize, usize)>,
    /// Where the preprocessing diagnostic reported last was folded or stored.
    last_other:   Option<usize>,
    /// The latest pending preprocessing error at each location.
    other_errors: HashMap<Vec<SourceVector>, usize>,
    errors:       usize,
    warnings:     usize,
}

/// One diagnostic awaiting rendering, with any later errors folded in.
struct PendingDiagnostic {
    diagnostic:        Diagnostic,
    location:          Vec<SourceVector>,
    ordering_location: Option<(u32, u32)>,
    /// Whether errors at the same place may be folded into this one.
    foldable:          bool,
}

impl PendingDiagnostic {
    fn absorbs(&self, location: &[SourceVector]) -> bool {
        self.foldable
            && self.diagnostic.severity == ErrorSeverity::Error
            && self.location == location
    }
}

impl DiagnosticReporter {
    fn new() -> Self {
        Self {
            renderer:     Renderer::new(RenderColor::for_stderr()),
            pending:      Vec::new(),
            last_parser:  None,
            last_other:   None,
            other_errors: HashMap::default(),
            errors:       0,
            warnings:     0,
        }
    }

    fn report(&mut self, error: &TranslationError, context: &mut Context<'_>) {
        let source = error.source_vectors(context);
        let diagnostic = error.to_diagnostic(context, source);
        let location = context.get_source_vectors(source).to_vec();
        let ordering_location = match error {
            | TranslationError::Parsing(error) => error.ordering_location,
            // Preprocessing errors may still point into a macro definition.
            | _ if diagnostic.severity != ErrorSeverity::Warning => None,
            | _ => location
                .first()
                .map(|source| (source.source_file_index, source.index)),
        };
        let (parser, consumed, foldable) = match error {
            | TranslationError::Parsing(error) => (true, error.consumed_tokens, error.may_fold()),
            | _ => (false, 0, true),
        };
        let target =
            if diagnostic.severity == ErrorSeverity::Error && foldable && !location.is_empty() {
                if parser {
                    self.last_parser
                        .filter(|&(index, last_consumed)| {
                            last_consumed == consumed && self.pending[index].absorbs(&location)
                        })
                        .map(|(index, _)| index)
                        .or_else(|| {
                            self.other_errors
                                .get(&location)
                                .copied()
                                .filter(|&index| self.pending[index].absorbs(&location))
                        })
                } else {
                    self.last_other
                        .filter(|&index| self.pending[index].absorbs(&location))
                }
            } else {
                None
            };
        let index = if let Some(index) = target {
            self.pending[index].diagnostic.absorb(diagnostic, context);
            index
        } else {
            if !parser && diagnostic.severity == ErrorSeverity::Error && !location.is_empty() {
                _ = self
                    .other_errors
                    .insert(location.clone(), self.pending.len());
            }
            self.pending.push(PendingDiagnostic {
                diagnostic,
                location,
                ordering_location,
                foldable,
            });
            self.pending.len() - 1
        };
        if parser {
            self.last_parser = Some((index, consumed));
        } else {
            self.last_other = Some(index);
        }
    }

    /// Lookahead can fetch a warning beyond the current parser error. Order
    /// each file run only after folding; preprocessing errors, file transitions
    /// and unknown locations
    /// remain barriers, and macro diagnostics use their captured invocation.
    fn order_source_runs(&mut self) {
        for run in self.pending.chunk_by_mut(|left, right| {
            left.ordering_location.is_some()
                && left.ordering_location.map(|(file, _)| file)
                    == right.ordering_location.map(|(file, _)| file)
        }) {
            run.sort_by_key(|diagnostic| diagnostic.ordering_location);
        }
        self.last_parser = None;
        self.last_other = None;
        self.other_errors.clear();
    }

    fn flush(&mut self, context: &Context<'_>) {
        for PendingDiagnostic { diagnostic, .. } in self.pending.drain(..) {
            match diagnostic.severity {
                | ErrorSeverity::Error => self.errors += 1,
                | ErrorSeverity::Warning => self.warnings += 1,
                | ErrorSeverity::Note => {},
            }
            eprint!("{}", self.renderer.render(&diagnostic, context));
        }
        self.last_parser = None;
        self.last_other = None;
        self.other_errors.clear();
    }

    fn finish(&mut self, context: &Context<'_>) {
        self.flush(context);
        let counts: Vec<String> = [(self.errors, "error"), (self.warnings, "warning")]
            .into_iter()
            .filter(|&(count, _)| count > 0)
            .map(|(count, noun)| count_of(count, noun))
            .collect();
        if !counts.is_empty() {
            eprintln!("{} generated.", counts.join(" and "));
        }
    }
}
