//! Directive dispatch and the non-conditional directives.
//!
//! C99: the `group-part`, `control-line`, and `non-directive` grammar of
//! §6.10 paragraph 1, pp. 145-146; PDF pp. 157-158 (also §A.3, pp. 416-418;
//! PDF pp. 428-430), and the directives `#include` (§6.10.2, pp. 149-151;
//! PDF pp. 161-163), `#define` (§6.10.3, pp. 151-153; PDF pp. 163-165),
//! `#undef` (§6.10.3.5, p. 155; PDF p. 167), `#line` (§6.10.4, p. 158; PDF
//! p. 170), `#error` (§6.10.5, p. 159; PDF p. 171), `#pragma` (§6.10.6,
//! p. 159; PDF p. 171), and the null directive (§6.10.7, p. 160; PDF
//! p. 172). Conditional directives are in `conditional`.
//!
//! Directive tokens are not macro-replaced unless a clause says so (§6.10
//! paragraph 7, p. 147; PDF p. 159). Of the directives here, only the
//! operands of `#include` and `#line` are; `#pragma` operands are not, which
//! footnote 152 permits (§6.10.6 paragraph 1, p. 159; PDF p. 171).

use super::language_features;
mod pragma;

use std::{
    ffi::OsStr,
    ops::ControlFlow,
    path::{
        Component,
        Path,
    },
};

use super::{
    Expander,
    LiteralUnit,
    errors::{
        PreprocessorError,
        PreprocessorErrorType,
    },
    runtime::{
        TokenizerFrame,
        TokenizerFrameType,
    },
    token::{
        StringTokenType,
        TokenType,
    },
};
use crate::{
    configuration::{
        ExtensionPolicy,
        Feature,
    },
    translation_phases::{
        Context,
        ErrorSeverity,
        SourcePosition,
        SourceVectors,
        TranslationError,
        TranslationPhase,
        preprocessor_tokenizer::{
            LogicalCharacter,
            PreprocessorToken,
            PreprocessorTokenType,
            logical_characters,
            position_after,
        },
    },
    util::bump::{
        ArenaString,
        ArenaVec,
        Bump,
    },
};

mod include;

mod definitions;

mod line;

mod error;

impl Expander<'_, '_, '_, '_> {
    /// Executes the directive that `token`, a `#`, introduces.
    ///
    /// C99: §6.10 paragraphs 1-3, pp. 145-147; PDF pp. 157-159. A `#` begins
    /// a directive only at the start of a line; one found elsewhere is
    /// diagnosed and, for recovery, still read as a directive. A name that is
    /// not a directive makes a `non-directive`, to which C99 gives no
    /// meaning; it is diagnosed and skipped, as GCC and Clang do.
    /// `# new-line` is the null directive (§6.10.7 paragraph 1, p. 160; PDF
    /// p. 172).
    pub(super) fn parse_directive(&mut self, token: PreprocessorToken) {
        if !self.last_was_newline {
            self.context.preprocessor_error(PreprocessorError {
                error_type:     PreprocessorErrorType::HashMustBeFirstCharacterOnLine,
                source_vectors: token.source_vectors,
            });
        }
        let Some(directive) = Self::next_ignore_whitespace(&mut self.tokenizer, self.context)
        else {
            return;
        };
        match directive.kind {
            // Null directive (C99 §6.10.7p1).
            | PreprocessorTokenType::Newline => {
                self.resume_at_line_start();
                return;
            },
            // This is the general case. We handle it in the function body.
            // If token is defined, it'll be handled when we match on contents.
            | PreprocessorTokenType::Defined
            | PreprocessorTokenType::Identifier
            | PreprocessorTokenType::UniversalIdentifier => (),
            | _ => {
                self.context.preprocessor_error(PreprocessorError {
                    error_type:     PreprocessorErrorType::HashMustBeFollowedByIdentifier,
                    source_vectors: directive.source_vectors,
                });
                self.skip_until_newline();
                return;
            },
        }
        match self.context.string_cache.at(directive.contents) {
            | "if" => self.parse_if_directive(directive),
            | "ifdef" => self.parse_ifdef_directive(directive),
            | "ifndef" => self.parse_ifndef_directive(directive),
            | "elif" => self.parse_elif_directive(directive),
            | name @ ("elifdef" | "elifndef")
                if self.context.configuration.accepts(Feature::Elifdef) =>
            {
                self.context.report_extension(
                    Feature::Elifdef,
                    if name == "elifdef" {
                        "#elifdef"
                    } else {
                        "#elifndef"
                    },
                    directive.source_vectors,
                );
                self.parse_elif_directive(directive);
            },
            | "else" => self.parse_else_directive(directive),
            | "endif" => self.parse_endif_directive(directive),
            | "include" => self.parse_include_directive(directive),
            | "include_next" => {
                self.context.report_extension(
                    Feature::IncludeNext,
                    "#include_next",
                    directive.source_vectors,
                );
                self.parse_include_directive(directive);
            },
            | "embed" if self.context.configuration.accepts(Feature::Embed) =>
                self.parse_embed_directive(directive),
            | "ident" | "sccs" => {
                self.context.report_extension(
                    Feature::IdentDirective,
                    "#ident/#sccs",
                    directive.source_vectors,
                );
                let operand = Self::next_ignore_whitespace(&mut self.tokenizer, self.context);
                if operand.is_none_or(|t| t.kind != PreprocessorTokenType::String) {
                    self.language_error(
                        "expected a string literal after #ident/#sccs",
                        directive.source_vectors,
                    );
                }
                if operand.is_none_or(|t| t.kind != PreprocessorTokenType::Newline) {
                    self.skip_until_newline();
                }
                self.resume_at_line_start();
            },
            | "define" => self.parse_define_directive(),
            | "undef" => self.parse_undef_directive(),
            | "line" => self.parse_line_directive(),
            | "error" => self.parse_error_directive(directive),
            | "warning"
                if self
                    .context
                    .configuration
                    .accepts(Feature::WarningDirective) =>
            {
                self.context.report_extension(
                    Feature::WarningDirective,
                    "#warning",
                    directive.source_vectors,
                );
                self.parse_error_directive(directive);
                self.resume_at_line_start();
            },
            | "pragma" => {
                let from = self
                    .context
                    .first_source_vector(directive.source_vectors)
                    .index;
                if !self.parse_pragma_directive(from) {
                    self.skip_until_newline();
                }
                self.resume_at_line_start();
            },
            | _ => {
                self.context.preprocessor_error(PreprocessorError {
                    error_type:     PreprocessorErrorType::UnknownDirective,
                    source_vectors: directive.source_vectors,
                });
                self.skip_until_newline();
            },
        }
    }
}
