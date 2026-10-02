//! BCC C compiler

#[cfg(test)]
#[doc(hidden)]
mod shut_up_clippy_about_unused_dev_dependencies {
    use criterion as _;
    use pretty_assertions as _;
    use proptest as _;
    use rstest as _;
}
// Only the benchmarking binary emits coz progress points.
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
#[cfg(all(unix, feature = "benchmarking-internals"))]
use coz as _;
use thiserror::Error;
#[cfg(test)]
use translation_phases::{
    TranslationPhase,
    parsing::ExternalDeclaration,
};
use translation_phases::{
    parsing::{
        InspectionOptions,
        Parser as LanguageParser,
    },
    preprocessing::Token,
};

#[cfg(feature = "benchmarking-internals")]
use crate::translation_phases::box_path_from_str;
use crate::{
    diagnostics::{
        ColorChoice as RenderColor,
        Diagnostic,
        Renderer,
        ToDiagnostic,
        c_quoted,
        count_of,
    },
    translation_phases::{
        Context,
        ErrorSeverity,
        GetSourceVectors,
        SourceVector,
        TranslationError,
        preprocessing::{
            CharacterTokenType,
            IntegerTokenType,
            Preprocessor,
            StringTokenType,
            TokenType,
        },
    },
    util::{
        read_to_string_lossy,
        shared::{
            SharedString,
            SharedVec,
        },
    },
};

pub(crate) mod configuration;
pub(crate) mod diagnostics;
pub(crate) mod float_parsing;
pub(crate) mod translation_phases;
pub(crate) mod util;

struct PreprocessorIterator {
    preprocessor:  Preprocessor,
    context:       Context,
    pending_token: Option<Token>,
}

impl PreprocessorIterator {
    fn new(
        source_filename: Box<Path>,
        input_string: SharedString,
        quote_include: SharedVec<PathBuf>,
        system_include: SharedVec<PathBuf>,
    ) -> Self {
        let mut context = Context::new();
        let preprocessor = Preprocessor::new(
            &mut context,
            source_filename,
            input_string,
            quote_include,
            system_include,
        );
        Self {
            preprocessor,
            context,
            pending_token: None,
        }
    }

    /// Whether the next [`Iterator::next`] call can discard preprocessor
    /// provenance that earlier items still reference.
    fn compacts_on_next(&self) -> bool {
        !self.context.has_pending_errors()
            && self.pending_token.is_none()
            && self.preprocessor.next_iterator_item_compacts()
    }
}

impl Iterator for PreprocessorIterator {
    type Item = Result<Token, TranslationError>;

    fn next(&mut self) -> Option<Self::Item> {
        if let Some(error) = self.context.pop_pending_error() {
            return Some(Err(error));
        }
        if let Some(token) = self.pending_token.take() {
            return Some(Ok(token));
        }

        let token = self.preprocessor.next_iterator_item(&mut self.context);
        if let Some(error) = self.context.pop_pending_error() {
            self.pending_token = token;
            Some(Err(error))
        } else {
            token.map(Ok)
        }
    }
}

#[cfg(test)]
struct ParserIterator {
    parser:       LanguageParser,
    context:      Context,
    pending_item: Option<ExternalDeclaration>,
}

#[cfg(test)]
impl ParserIterator {
    fn new(
        source_filename: Box<Path>,
        input_string: SharedString,
        quote_include: SharedVec<PathBuf>,
        system_include: SharedVec<PathBuf>,
    ) -> Self {
        let mut context = Context::new();
        let preprocessor = Preprocessor::new(
            &mut context,
            source_filename,
            input_string,
            quote_include,
            system_include,
        );
        Self {
            parser: LanguageParser::new(preprocessor),
            context,
            pending_item: None,
        }
    }
}

#[cfg(test)]
impl Iterator for ParserIterator {
    type Item = Result<ExternalDeclaration, TranslationError>;

    fn next(&mut self) -> Option<Self::Item> {
        if let Some(error) = self.context.pop_pending_error() {
            return Some(Err(error));
        }
        if let Some(item) = self.pending_item.take() {
            return Some(Ok(item));
        }

        let item = self.parser.next_item(&mut self.context);
        if let Some(error) = self.context.pop_pending_error() {
            self.pending_item = item;
            Some(Err(error))
        } else {
            item.map(Ok)
        }
    }
}

#[cfg(test)]
mod pipeline_iterator_tests {
    use super::*;
    use crate::{
        configuration::{
            CStandard,
            CompilerConfiguration,
            ExtensionPolicy,
        },
        translation_phases::preprocessing::{
            PreprocessorError,
            PreprocessorErrorType,
        },
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
            "error: expected `,`, `=`, `;`, or a function body after the declarator, found \
             identifier `extra`",
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
}

#[cfg(test)]
mod syntax_tree_consumer_tests {
    use super::*;
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

        let ExternalDeclaration::Declaration(function_root) = unit.external_declarations()[1]
        else {
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
}

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
    let (input_string, source_filename) =
        match (args.input.input.take(), args.input.input_file.take()) {
            | (Some(input), None) => (
                SharedString::from(input),
                PathBuf::from("<input>").into_boxed_path(),
            ),
            | (None, Some(input_file)) => (
                SharedString::from(read_to_string_lossy(&input_file).map_err(|source| {
                    MainError::OpenInputFileError {
                        path: input_file.clone(),
                        source,
                    }
                })?),
                input_file.into_boxed_path(),
            ),
            | _ => unreachable!("clap requires exactly one input source"),
        };
    // GCC searches `CPATH` like `-I` (before `-isystem`) and
    // `C_INCLUDE_PATH` like a trailing `-isystem`.
    let mut system_include = include_path_from_env("CPATH");
    system_include.append(&mut args.system_include);
    system_include.extend(include_path_from_env("C_INCLUDE_PATH"));
    args.system_include = system_include;

    if args.output.tokens {
        print_preprocessor_output(
            source_filename,
            input_string,
            args.quote_include.into(),
            args.system_include.into(),
        );
    } else {
        print_parser_output(
            source_filename,
            input_string,
            args.quote_include.into(),
            args.system_include.into(),
            &args.output.parser,
            !args.no_repeated_specifier_warnings,
        );
    }
    Ok(())
}

fn print_preprocessor_output(
    source_filename: Box<Path>,
    input_string: SharedString,
    quote_include: SharedVec<PathBuf>,
    system_include: SharedVec<PathBuf>,
) {
    let mut reporter = DiagnosticReporter::new();
    let mut iterator =
        PreprocessorIterator::new(source_filename, input_string, quote_include, system_include);
    loop {
        // A deferred diagnostic's labels index the preprocessor arena, which
        // the next poll may compact; render it while its provenance is live.
        if iterator.compacts_on_next() {
            reporter.flush(&iterator.context);
        }
        let Some(item) = iterator.next() else {
            break;
        };
        match item {
            | Ok(token) => {
                reporter.flush(&iterator.context);
                eprintln!("{}", describe_token(token, &iterator.context));
            },
            | Err(error) => reporter.report(&error, &mut iterator.context),
        }
    }
    reporter.finish(&iterator.context);
}

/// One line per token: its location, kind, source spelling, and for
/// constants the value and type the preprocessor assigned.
fn describe_token(token: Token, context: &Context) -> String {
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
            c_quoted("", '"', context.string_cache.at(contents))
        ),
        | TokenType::String(StringTokenType::WideString(contents)) => format!(
            "wide string literal {}",
            c_quoted("L", '"', context.string_cache.at(contents))
        ),
        | TokenType::Character(character) => {
            let (value, type_name) = match character {
                | CharacterTokenType::Char(c) => (i64::from(u32::from(c)), "int"),
                | CharacterTokenType::WideChar(c) => (i64::from(u32::from(c)), "wchar_t"),
                | CharacterTokenType::MultiChar(value) => (i64::from(value), "int"),
            };
            format!("character constant `{spelling}` = {value} ({type_name})")
        },
        | TokenType::Integer(integer) => {
            let (value, type_name) = match integer {
                | IntegerTokenType::Int(value) => (i128::from(value), "int"),
                | IntegerTokenType::Long(value) => (i128::from(value), "long"),
                | IntegerTokenType::LongLong(value) => (i128::from(value), "long long"),
                | IntegerTokenType::UnsignedInt(value) => (i128::from(value), "unsigned int"),
                | IntegerTokenType::UnsignedLong(value) => (i128::from(value), "unsigned long"),
                | IntegerTokenType::UnsignedLongLong(value) =>
                    (i128::from(value), "unsigned long long"),
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

fn print_parser_output(
    source_filename: Box<Path>,
    input_string: SharedString,
    quote_include: SharedVec<PathBuf>,
    system_include: SharedVec<PathBuf>,
    output: &ParserOutput,
    repeated_specifier_warnings: bool,
) {
    let mut context = Context::new();
    context.configuration = context
        .configuration
        .with_repeated_specifier_warnings(repeated_specifier_warnings);
    let preprocessor = Preprocessor::new(
        &mut context,
        source_filename,
        input_string,
        quote_include,
        system_include,
    );
    let unit = LanguageParser::new(preprocessor).parse_translation_unit(&mut context);
    let mut reporter = DiagnosticReporter::new();
    while let Some(error) = context.pop_pending_error() {
        reporter.report(&error, &mut context);
    }
    reporter.flush(&context);

    if output.syntax_tree {
        eprint!(
            "{}",
            unit.syntax().inspect(
                unit.external_declarations(),
                &context,
                InspectionOptions {
                    show_locations: output.syntax_locations,
                },
            )
        );
    }
    if output.raw_syntax {
        eprintln!("{:#?}", unit.syntax().raw_debug());
    }
    reporter.finish(&context);
}

/// Renders diagnostics to stderr and summarizes them at the end, like
/// `N errors and M warnings generated`.
///
/// An error reported at exactly the same place as the error before it is a
/// cascade from the same mistake, so it is folded into that error rather
/// than printed again.
struct DiagnosticReporter {
    renderer: Renderer,
    pending:  Option<(Diagnostic, Vec<SourceVector>)>,
    errors:   usize,
    warnings: usize,
}

impl DiagnosticReporter {
    fn new() -> Self {
        Self {
            renderer: Renderer::new(RenderColor::for_stderr()),
            pending:  None,
            errors:   0,
            warnings: 0,
        }
    }

    fn report(&mut self, error: &TranslationError, context: &mut Context) {
        let source = error.source_vectors(context);
        let diagnostic = error.to_diagnostic(context, source);
        let location = context.get_source_vectors(source).to_vec();
        if let Some((pending, pending_location)) = &mut self.pending
            && diagnostic.severity == ErrorSeverity::Error
            && pending.severity == ErrorSeverity::Error
            && !location.is_empty()
            && *pending_location == location
        {
            pending.absorb(diagnostic, context);
            return;
        }
        self.flush(context);
        self.pending = Some((diagnostic, location));
    }

    fn flush(&mut self, context: &Context) {
        let Some((diagnostic, _)) = self.pending.take() else {
            return;
        };
        match diagnostic.severity {
            | ErrorSeverity::Error => self.errors += 1,
            | ErrorSeverity::Warning => self.warnings += 1,
            | ErrorSeverity::Note => {},
        }
        eprint!("{}", self.renderer.render(&diagnostic, context));
    }

    fn finish(&mut self, context: &Context) {
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

#[doc(hidden)]
#[cfg(feature = "benchmarking-internals")]
pub fn preprocess_one_million() -> usize {
    let million_lines = one_million_lines();
    let iterator = PreprocessorIterator::new(
        box_path_from_str("<input>"),
        million_lines.to_owned().into(),
        SharedVec::default(),
        SharedVec::default(),
    );
    iterator.count()
}

#[doc(hidden)]
#[cfg(feature = "benchmarking-internals")]
pub fn one_million_input_bytes() -> u64 {
    u64::try_from(one_million_lines().len()).expect("benchmark input length must fit in u64")
}

#[cfg(feature = "benchmarking-internals")]
fn one_million_lines() -> &'static str {
    include_str!(concat!(env!("OUT_DIR"), "/one-million-lines.c"))
}

#[cfg(feature = "benchmarking-internals")]
#[expect(
    clippy::large_include_file,
    reason = "The generated parser benchmark input is intentionally large."
)]
fn parser_mix() -> &'static str {
    include_str!(concat!(env!("OUT_DIR"), "/parser-mix.c"))
}

/// Summary of one benchmarked parse, returned so the work cannot be elided
/// and so benchmark setup can reject inputs that produce diagnostics.
#[doc(hidden)]
#[cfg(feature = "benchmarking-internals")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ParseBenchmarkSummary {
    pub external_declarations: usize,
    pub diagnostics:           usize,
}

#[cfg(feature = "benchmarking-internals")]
fn parse_benchmark_input(input: &'static str) -> ParseBenchmarkSummary {
    let mut context = Context::new();
    let preprocessor = Preprocessor::new(
        &mut context,
        box_path_from_str("<input>"),
        input.to_owned().into(),
        SharedVec::default(),
        SharedVec::default(),
    );
    let unit = LanguageParser::new(preprocessor).parse_translation_unit(&mut context);
    ParseBenchmarkSummary {
        external_declarations: unit.external_declarations().len(),
        diagnostics:           context.take_pending_errors().len(),
    }
}

#[doc(hidden)]
#[cfg(feature = "benchmarking-internals")]
#[must_use]
pub fn parse_one_million() -> ParseBenchmarkSummary {
    parse_benchmark_input(one_million_lines())
}

#[doc(hidden)]
#[cfg(feature = "benchmarking-internals")]
#[must_use]
pub fn parse_mix() -> ParseBenchmarkSummary {
    parse_benchmark_input(parser_mix())
}

#[doc(hidden)]
#[cfg(feature = "benchmarking-internals")]
#[must_use]
pub fn parser_mix_input_bytes() -> u64 {
    u64::try_from(parser_mix().len()).expect("benchmark input length must fit in u64")
}

#[doc(hidden)]
#[cfg(feature = "benchmarking-internals")]
#[must_use]
pub fn parser_mix_input_lines() -> u64 {
    u64::try_from(parser_mix().lines().count()).expect("benchmark line count must fit in u64")
}
