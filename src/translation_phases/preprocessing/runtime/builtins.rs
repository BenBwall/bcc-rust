//! Expand predefined macros and execute the pragma operator.
//!
//! C99: translation phase 4, §5.1.1.2 paragraph 1 item 4, p. 10; PDF p. 22;
//! predefined macros, §6.10.8 paragraph 1, p. 160; PDF p. 172;
//! pragma operator, §6.10.9 paragraph 1, p. 161; PDF p. 173;
//! string spelling, §6.4.5 paragraphs 1 and 3, p. 62; PDF p. 74.
//! Pragma payloads are passed to directive handling; dialect builtins are
//! delegated.

use std::{
    fmt::Write,
    mem::take,
    ops::ControlFlow,
    path::Path,
};

use chrono::Local;

use super::super::{
    Expander,
    PreprocessorError,
    PreprocessorErrorType,
};
use crate::{
    translation_phases::{
        Context,
        TranslationPhase,
        preprocessor_tokenizer::{
            PreprocessorToken,
            PreprocessorTokenType,
            TokenSource,
        },
    },
    util::{
        bump::{
            ArenaString,
            Bump,
        },
        string_cache::StringCacheId,
    },
};

impl Expander<'_, '_, '_, '_> {
    /// Expands a registered builtin at its invocation location. A consumed
    /// pragma returns no token; the caller resumes its frame reader.
    /// C99: §6.10.8p1 and §6.10.9p1, pp. 160-161; PDF pp. 172-173.
    pub(in crate::translation_phases::preprocessing) fn expand_builtin(
        &mut self,
        token: PreprocessorToken,
    ) -> Option<PreprocessorToken> {
        match self.context.string_cache.at(token.contents) {
            // C99 §6.10.8p1: each built-in expands to an ordinary
            // token spelled as C source, located at the invocation.
            | "__FILE__" => {
                let invocation = self.invocation_location(token);
                let file = self.context.get_source_file(invocation.source_file_index);
                let contents =
                    intern_string_literal(self.context, self.scratch, &file.to_string_lossy());
                Some(PreprocessorToken {
                    kind: PreprocessorTokenType::String,
                    contents,
                    source_vectors: self.context.push_source_vectors(&[invocation]),
                })
            },
            | "__LINE__" => {
                let invocation = self.invocation_location(token);
                let contents = intern_line_number(self.context, self.scratch, invocation.line);
                Some(PreprocessorToken {
                    kind: PreprocessorTokenType::Number,
                    contents,
                    source_vectors: self.context.push_source_vectors(&[invocation]),
                })
            },
            | name @ ("__STDC__"
            | "__STDC_VERSION__"
            | "__STRICT_ANSI__"
            | "__STDC_HOSTED__"
            | "__STDC_MB_MIGHT_NEQ_WC__") => {
                // C99 §6.10.8p1. `__STDC_HOSTED__` follows the
                // configured execution environment (§4p6, §5.1.2):
                // 1 by default, 0 with `-ffreestanding`. The version
                // must retain its prescribed long suffix.
                // MB_MIGHT_NEQ_WC permits unequal codes; its 1
                // does not assert that their values differ.
                let spelling = if name == "__STDC_VERSION__" {
                    self.context
                        .configuration
                        .standard()
                        .version_macro()
                        .expect("version built-in is registered only when defined")
                } else if name == "__STDC_HOSTED__" && !self.context.configuration.hosted() {
                    "0\0"
                } else {
                    "1\0"
                };
                Some(PreprocessorToken {
                    kind:           PreprocessorTokenType::Number,
                    contents:       self.context.string_cache.intern(spelling),
                    source_vectors: token.source_vectors,
                })
            },
            | name @ ("__DATE__" | "__TIME__") => {
                let is_date = name == "__DATE__";
                let source_date_epoch = self.context.configuration.source_date_epoch();
                let timestamp = self.state.translation_timestamp.get_or_insert_with(|| {
                    TranslationTimestamp::new(self.state.arena, source_date_epoch)
                });
                let contents = intern_string_literal(
                    self.context,
                    self.scratch,
                    if is_date {
                        &timestamp.date
                    } else {
                        &timestamp.time
                    },
                );
                Some(PreprocessorToken {
                    kind: PreprocessorTokenType::String,
                    contents,
                    source_vectors: token.source_vectors,
                })
            },
            // C99 §6.10.9p1: `_Pragma ( string-literal )` runs
            // the destringized literal, retokenized, as the
            // pp-tokens of a `#pragma`, and all four tokens
            // are removed.
            | "_Pragma" => {
                let from = self.invocation_location(token).index;
                self.expand_pragma_operator(from);
                None
            },
            | name if super::language_features::LANGUAGE_BUILTINS
                .iter()
                .any(|(spelling, _)| *spelling == name) =>
                self.language_builtin(token),
            | s =>
                unreachable!("Compiler bug: Predefined macro {s:#?} not in PREDEFINED_MACRO_NAMES"),
        }
    }

    /// Executes the C99 string-form pragma operator.
    /// C99: §6.10.9p1, p. 161; PDF p. 173.
    fn expand_pragma_operator(&mut self, from: u32) {
        _ =
            self.expect_token_preserving_rejected::<true>(
                |_, t| t.kind == PreprocessorTokenType::OpeningParenthesis,
                |_, token| {
                    ControlFlow::Break(PreprocessorError {
                        error_type:
                            PreprocessorErrorType::MissingOpeningParenthesisInPragmaOperator(
                                token.kind,
                            ),
                        source_vectors: token.source_vectors,
                    })
                },
                "parsing pragma operator",
            );

        let Some(string_token) = self.expect_token_preserving_rejected::<true>(
            |_, t| t.kind == PreprocessorTokenType::String,
            |_, token| {
                ControlFlow::Break(PreprocessorError {
                    error_type:     PreprocessorErrorType::MissingStringLiteralInPragmaOperator(
                        token.kind,
                    ),
                    source_vectors: token.source_vectors,
                })
            },
            "parsing pragma operator",
        ) else {
            return;
        };

        let input = Self::prepare_pragma_operator_string(self.context, string_token.contents);

        let tokenizer = take(&mut self.tokenizer);
        // Each operator gets its own identity:
        // diagnostics rendered later must quote this
        // payload, not the most recent one.
        let pragma_string = self
            .context
            .add_synthetic_source_file(Path::new("<pragma string>"), input);
        self.tokenizer = TokenSource::new(self.context, self.scratch, pragma_string, input);
        self.pushed_frames += 1;
        _ = self.parse_pragma_directive(from);
        if self.tokenizer.next_item(self.context).is_some() {
            let source_vectors = self.current_location();
            self.context.preprocessor_error(PreprocessorError {
                error_type: PreprocessorErrorType::ExtraTokensAfterPragmaOperator,
                source_vectors,
            });
        }
        self.tokenizer = tokenizer;

        _ =
            self.expect_token_preserving_rejected::<true>(
                |_, t| t.kind == PreprocessorTokenType::ClosingParenthesis,
                |_, token| {
                    ControlFlow::Break(PreprocessorError {
                        error_type:
                            PreprocessorErrorType::MissingClosingParenthesisInPragmaOperator(
                                token.kind,
                            ),
                        source_vectors: token.source_vectors,
                    })
                },
                "parsing pragma operator",
            );
    }
}

/// The date and time of translation, spelled as C99 §6.10.8p1 requires.
///
/// C99: §6.10.8 paragraph 1, p. 160; PDF p. 172: `"Mmm dd yyyy"` with a
/// space before a day below 10, and `"hh:mm:ss"`. The values stay constant
/// for the translation unit (§6.10.8 paragraph 3, p. 161; PDF p. 173).
#[derive(Debug)]
pub(in crate::translation_phases::preprocessing) struct TranslationTimestamp<'pp> {
    pub(in crate::translation_phases::preprocessing) date: ArenaString<'pp>,
    pub(in crate::translation_phases::preprocessing) time: ArenaString<'pp>,
}

/// Spells `value` as a narrow C string literal whose evaluated contents are
/// exactly `value`, passing the spelling to `write` one character at a time.
///
/// C99: §6.4.5 paragraphs 1 and 3, p. 62; PDF p. 74: an `s-char` excludes
/// `"`, `\`, and new-line, which need escape sequences.
pub(in crate::translation_phases::preprocessing) fn spell_string_literal(
    value: &str,
    mut write: impl FnMut(char),
) {
    write('"');
    for c in value.chars() {
        match c {
            | '\\' | '"' => {
                write('\\');
                write(c);
            },
            | '\n' => {
                write('\\');
                write('n');
            },
            | _ => write(c),
        }
    }
    write('"');
}

/// Interns the string literal that [`spell_string_literal`] spells for
/// `value`, building it in `scratch`, which it leaves as it was when nothing
/// else allocated there meanwhile.
fn intern_string_literal(context: &mut Context<'_>, scratch: &Bump, value: &str) -> StringCacheId {
    let mut spelling = ArenaString::new_in(scratch);
    spell_string_literal(value, |c| spelling.push(c));
    context.string_cache.intern(&*spelling)
}

/// Interns the spelling of the number `line`, building it in `scratch`, which
/// it leaves as it was when nothing else allocated there meanwhile. Number
/// spellings carry the trailing NUL that numeric conversion expects.
fn intern_line_number(context: &mut Context<'_>, scratch: &Bump, line: u32) -> StringCacheId {
    let mut spelling = ArenaString::new_in(scratch);
    write!(spelling, "{line}\0").expect("arena formatting cannot fail");
    context.string_cache.intern(&*spelling)
}

impl<'pp> TranslationTimestamp<'pp> {
    /// Spells `source_date_epoch`, seconds since the Unix epoch, in UTC, so
    /// builds can pin the expansion (the CLI takes it from `SOURCE_DATE_EPOCH`,
    /// as GCC and Clang do). Without it, or for seconds that no date can
    /// represent, spells the local time.
    ///
    /// A pinned value stands in for the actual time of translation that
    /// C99 §6.10.8p1 names; that is GCC's and Clang's choice too.
    pub(in crate::translation_phases::preprocessing) fn new(
        pp: &'pp Bump,
        source_date_epoch: Option<i64>,
    ) -> Self {
        let time = source_date_epoch
            .and_then(|seconds| chrono::DateTime::from_timestamp(seconds, 0))
            .map_or_else(|| Local::now().naive_local(), |time| time.naive_utc());
        let mut date = ArenaString::new_in(pp);
        let mut clock = ArenaString::new_in(pp);
        write!(date, "{}", time.format("%b %e %Y")).expect("arena formatting cannot fail");
        write!(clock, "{}", time.format("%H:%M:%S")).expect("arena formatting cannot fail");
        Self { date, time: clock }
    }
}
