//! `#pragma GCC diagnostic` and `#pragma clang diagnostic`, and their
//! `_Pragma` forms. Clang reads both spellings, with `fatal` besides; GCC
//! reads its own and takes the other for an unknown pragma, and TCC and `cl`
//! read neither. Operands are not macro-replaced in either compiler.
//!
//! This reads the line and says what it means; the table of what changed where
//! is [`crate::translation_phases::Context::configuration_at`]'s.
//!
//! GCC: <https://gcc.gnu.org/onlinedocs/gcc/Diagnostic-Pragmas.html>.
//! Clang: <https://clang.llvm.org/docs/UsersManual.html#controlling-diagnostics-via-pragmas>.
//! C99: implementation-defined pragma behavior, §6.10.6 paragraph 1, p. 159;
//! PDF p. 171; the `_Pragma` operator, §6.10.9 paragraph 1, p. 161; PDF p. 173;
//! string literal concatenation, §6.4.5 paragraph 4, p. 62; PDF p. 74.

use super::super::super::{
    Expander,
    errors::{
        PragmaDiagnosticProblem as Problem,
        PreprocessorError,
        PreprocessorErrorType,
    },
};
use crate::{
    configuration::{
        CompilerIdentity,
        DiagnosticGroup,
        PragmaClass,
        RuleSet,
        clang_knows_warning,
        gcc_knows_flag,
        gcc_knows_warning,
    },
    translation_phases::{
        SourceVectors,
        preprocessor_tokenizer::{
            PreprocessorToken,
            PreprocessorTokenType,
        },
    },
    util::bump::{
        ArenaString,
        ArenaVec,
    },
};

impl<'tu> Expander<'_, 'tu, '_, '_> {
    /// `#pragma GCC diagnostic`, read up to `diagnostic`. Consumes the line.
    pub(super) fn pragma_diagnostic(&mut self) {
        let identity = self.context.configuration.compiler_identity();
        let (tokens, end) = self.pragma_line_tokens();
        match identity {
            | CompilerIdentity::Gcc => self.gcc_diagnostic_pragma(tokens, end),
            | CompilerIdentity::Tcc | CompilerIdentity::Msvc => {},
            | CompilerIdentity::Clang | CompilerIdentity::ClangCl =>
                self.clang_diagnostic_pragma(tokens, end),
        }
    }

    /// `#pragma clang diagnostic`, read up to `clang`, whose token is
    /// `pragma`: Clang's own, which GCC takes for an unknown pragma. Consumes
    /// the line.
    pub(super) fn pragma_diagnostic_namespace(&mut self, pragma: PreprocessorToken) {
        let identity = self.context.configuration.compiler_identity();
        if identity.rules() == RuleSet::Clang {
            let diagnostic = Self::next_ignore_whitespace(&mut self.tokenizer, self.context);
            debug_assert!(diagnostic.is_some(), "the caller looked at this token");
            let (tokens, end) = self.pragma_line_tokens();
            self.clang_diagnostic_pragma(tokens, end);
            return;
        }
        if identity == CompilerIdentity::Gcc
            && self
                .context
                .group_severity(DiagnosticGroup::UnknownPragmas, pragma.source_vectors)
                .is_some()
        {
            self.context.preprocessor_error(PreprocessorError {
                error_type:     PreprocessorErrorType::UnknownPragmaDirective,
                source_vectors: pragma.source_vectors,
            });
        }
        _ = self.pragma_line_tokens();
    }

    /// The rest of the pragma's line, unreplaced, and where it ends.
    fn pragma_line_tokens(&mut self) -> (&'tu [PreprocessorToken], SourceVectors) {
        let mut tokens = ArenaVec::new_in(self.context.tu_arena());
        let line_state = (self.last_was_newline, self.current_is_newline);
        let mut end = None;
        while let Some(token) = Self::next_ignore_whitespace(&mut self.tokenizer, self.context) {
            if token.kind == PreprocessorTokenType::Newline {
                end = Some(token.source_vectors);
                break;
            }
            tokens.push(token);
        }
        (self.last_was_newline, self.current_is_newline) = line_state;
        let end = end.unwrap_or_else(|| self.current_location());
        (tokens.leak(), end)
    }

    /// GCC's reading: it stops looking once it has what it needs, and warns
    /// under `-Wpragmas` about what it does not understand.
    fn gcc_diagnostic_pragma(&mut self, tokens: &[PreprocessorToken], end: SourceVectors) {
        let Some(&kind) = tokens.first() else {
            self.pragma_diagnostic_problem(Problem::GccMissingKind, end);
            return;
        };
        let spelling = kind
            .kind
            .is_identifier()
            .then(|| self.context.string_cache.at(kind.contents));
        let class = match spelling {
            | Some("push") => {
                self.context.pragma_diagnostic_push();
                return;
            },
            | Some("pop") => {
                _ = self.context.pragma_diagnostic_pop();
                return;
            },
            | Some("ignored_attributes") => {
                if tokens.len() == 1 {
                    self.pragma_diagnostic_problem(Problem::GccMissingAttribute, end);
                }
                return;
            },
            | Some("ignored") => PragmaClass::Ignored,
            | Some("warning") => PragmaClass::Warning,
            | Some("error") => PragmaClass::Error,
            | _ => {
                self.pragma_diagnostic_problem(Problem::GccExpectedKind, kind.source_vectors);
                return;
            },
        };
        let operands = &tokens[1..];
        let (literals, _) = self.pragma_option_literals(operands);
        let Some(first) = literals.first() else {
            let at = operands.first().map_or(end, |token| token.source_vectors);
            self.pragma_diagnostic_problem(Problem::GccMissingOption, at);
            return;
        };
        let at = first.source_vectors;
        let mut text = ArenaString::new_in(self.scratch);
        for &literal in literals {
            self.append_deprecation_message(literal, &mut text);
        }
        // GCC looks up the option without its first character: `"Wundef"`
        // finds its `-undef`, an option, though no warning's.
        let rest = text.get(1..).unwrap_or("");
        let warning = rest.strip_prefix('W');
        let controls = warning.is_some_and(|name| {
            name.is_empty()
                || (!name.starts_with("error=") && gcc_knows_warning(&text))
                || is_bcc_warning(name)
        });
        if !controls {
            let known = warning.is_some_and(|name| name.starts_with("error="))
                || gcc_knows_flag(&text)
                || rest.starts_with('R')
                || matches!(rest, "pedantic" | "pedantic-errors" | "undef" | "w");
            let problem = if known {
                Problem::GccNotWarningOption(self.context.diagnostic_text(&text))
            } else {
                Problem::GccUnknownOption
            };
            self.pragma_diagnostic_problem(problem, at);
            return;
        }
        self.context
            .pragma_diagnostic_class(warning.unwrap_or(""), class, false);
    }

    /// Clang's reading: anything after the option cancels the pragma, with a
    /// warning under `-Wunknown-pragmas`, and a missing option is an error.
    fn clang_diagnostic_pragma(&mut self, tokens: &[PreprocessorToken], end: SourceVectors) {
        let Some(&kind) = tokens.first() else {
            self.pragma_diagnostic_problem(Problem::ClangInvalidKind, end);
            return;
        };
        let after = tokens.get(1).map_or(end, |token| token.source_vectors);
        let spelling = kind
            .kind
            .is_identifier()
            .then(|| self.context.string_cache.at(kind.contents));
        let (class, fatal) = match spelling {
            | Some("push") => {
                self.context.pragma_diagnostic_push();
                if tokens.len() > 1 {
                    self.pragma_diagnostic_problem(Problem::ClangUnexpectedToken, after);
                }
                return;
            },
            | Some("pop") => {
                if !self.context.pragma_diagnostic_pop() {
                    self.pragma_diagnostic_problem(Problem::ClangCouldNotPop, after);
                }
                if tokens.len() > 1 {
                    self.pragma_diagnostic_problem(Problem::ClangUnexpectedToken, after);
                }
                return;
            },
            | Some("ignored") => (PragmaClass::Ignored, false),
            | Some("warning") => (PragmaClass::Warning, false),
            | Some("error") => (PragmaClass::Error, false),
            | Some("fatal") => (PragmaClass::Error, true),
            | Some(_) => {
                self.pragma_diagnostic_problem(Problem::ClangInvalidKind, after);
                return;
            },
            | None => {
                self.pragma_diagnostic_problem(Problem::ClangInvalidKind, kind.source_vectors);
                return;
            },
        };
        let (literals, rest) = self.pragma_option_literals(&tokens[1..]);
        let Some(first) = literals.first() else {
            self.pragma_diagnostic_problem(Problem::ExpectedString, after);
            return;
        };
        let at = first.source_vectors;
        if let Some(extra) = rest.first() {
            self.pragma_diagnostic_problem(Problem::ClangUnexpectedToken, extra.source_vectors);
            return;
        }
        let mut text = ArenaString::new_in(self.scratch);
        for &literal in literals {
            self.append_deprecation_message(literal, &mut text);
        }
        if text.len() < 3 || !(text.starts_with("-W") || text.starts_with("-R")) {
            self.pragma_diagnostic_problem(Problem::ClangInvalidOption, at);
            return;
        }
        // Remarks are not warnings; bcc has none.
        let Some(name) = text.strip_prefix("-W") else {
            return;
        };
        // The list holds each group's `no-` form too, which a pragma cannot
        // use.
        let known = name == "everything"
            || (!name.starts_with("no-") && clang_knows_warning(&text))
            || is_bcc_warning(name);
        if !known {
            let option = self.context.diagnostic_text(&text);
            self.pragma_diagnostic_problem(Problem::ClangUnknownGroup(option), at);
            return;
        }
        self.context.pragma_diagnostic_class(name, class, fatal);
    }

    /// `#pragma warning(...)`, read up to `warning`, as `cl` and clang-cl read
    /// it: `push` with an optional level, `pop`, or specifiers each with a
    /// list of warning numbers, `;` between them. Consumes the line.
    ///
    /// clang-cl's `push` and `pop` are its diagnostic pragmas' and it applies
    /// no number, since its warnings have none. `cl` applies the numbers bcc
    /// gives a warning (`Context::pragma_warning_number`) and says nothing
    /// about a line it cannot read, whose warnings bcc does not model.
    /// Microsoft Learn: "warning pragma".
    /// C99: implementation-defined pragma behavior, §6.10.6 paragraph 1,
    /// p. 159; PDF p. 171.
    pub(super) fn pragma_warning(&mut self) {
        let (tokens, end) = self.pragma_line_tokens();
        let problem = self.read_pragma_warning(tokens);
        if let Err((message, at)) = problem {
            let at = at.map_or(end, |token: PreprocessorToken| token.source_vectors);
            if self.context.configuration.compiler_identity() != CompilerIdentity::Msvc {
                self.pragma_diagnostic_problem(Problem::ClangWarning(message), at);
            }
        }
    }

    /// Reads and applies the operands of `#pragma warning`; the error is what
    /// was expected, and the token found instead, if any.
    fn read_pragma_warning(
        &mut self,
        tokens: &[PreprocessorToken],
    ) -> Result<(), (&'static str, Option<PreprocessorToken>)> {
        const KINDS: &str = "#pragma warning expected `push`, `pop`, `default`, `disable`, \
                             `error`, `once`, `suppress`, 1, 2, 3, or 4";
        let cl = self.context.configuration.compiler_identity() == CompilerIdentity::Msvc;
        let mut rest = tokens.iter().copied();
        let open = rest.next();
        if !open.is_some_and(|token| token.kind == PreprocessorTokenType::OpeningParenthesis) {
            return Err(("#pragma warning expected `(`", open));
        }
        let mut token = rest.next();
        let name = token
            .filter(|token| token.kind.is_identifier())
            .map(|token| self.context.string_cache.at(token.contents));
        match name {
            | Some("push") => {
                self.context.pragma_diagnostic_push();
                token = rest.next();
                if token.is_some_and(|token| token.kind == PreprocessorTokenType::Comma) {
                    let level = rest.next();
                    let number = level
                        .filter(|level| level.kind == PreprocessorTokenType::Number)
                        .map(|level| self.pragma_spelling(level));
                    token = if number.is_some() { rest.next() } else { level };
                    if !matches!(number, Some("0" | "1" | "2" | "3" | "4")) {
                        return Err((
                            "#pragma warning(push, level) requires a level between 0 and 4",
                            token,
                        ));
                    }
                }
            },
            | Some("pop") => {
                if !self.context.pragma_diagnostic_pop() {
                    let at = match rest.clone().next() {
                        | Some(token) => token.source_vectors,
                        | None => self.current_location(),
                    };
                    self.pragma_diagnostic_problem(Problem::ClangCouldNotPop, at);
                }
                token = rest.next();
            },
            | _ => loop {
                let number = token.filter(|token| token.kind == PreprocessorTokenType::Number);
                let specifier = name_or_level(token.map(|token| {
                    if token.kind.is_identifier() || number.is_some() {
                        self.context.string_cache.at(token.contents)
                    } else {
                        ""
                    }
                }));
                let Some(specifier) = specifier else {
                    return Err((KINDS, token));
                };
                let colon = rest.next();
                if !colon.is_some_and(|token| token.kind == PreprocessorTokenType::Colon) {
                    return Err(("#pragma warning expected `:`", colon));
                }
                token = rest.next();
                while let Some(number) =
                    token.filter(|token| token.kind == PreprocessorTokenType::Number)
                {
                    if cl && let Ok(code) = self.pragma_spelling(number).parse() {
                        self.context.pragma_warning_number(code, specifier);
                    }
                    token = rest.next();
                }
                if !token.is_some_and(|token| token.kind == PreprocessorTokenType::SemiColon) {
                    break;
                }
                token = rest.next();
            },
        }
        if !token.is_some_and(|token| token.kind == PreprocessorTokenType::ClosingParenthesis) {
            return Err(("#pragma warning expected `)`", token));
        }
        if let Some(extra) = rest.next() {
            self.pragma_warning_extra_tokens(extra);
        }
        Ok(())
    }

    fn pragma_warning_extra_tokens(&mut self, extra: PreprocessorToken) {
        if self.context.configuration.compiler_identity() != CompilerIdentity::Msvc {
            self.pragma_diagnostic_problem(Problem::ClangWarningExtraTokens, extra.source_vectors);
        }
    }

    fn pragma_spelling(&self, token: PreprocessorToken) -> &str {
        self.context
            .string_cache
            .at(token.contents)
            .trim_end_matches('\0')
    }

    /// The ordinary string literals that begin `tokens`, and what follows.
    fn pragma_option_literals<'a>(
        &self,
        tokens: &'a [PreprocessorToken],
    ) -> (&'a [PreprocessorToken], &'a [PreprocessorToken]) {
        let count = tokens
            .iter()
            .take_while(|token| self.is_deprecation_message_literal(**token))
            .count();
        tokens.split_at(count)
    }

    /// Reports `problem` under its group. GCC's `-E` reads these pragmas only
    /// for the settings of its own warnings, and says nothing of a bad one.
    fn pragma_diagnostic_problem(&mut self, problem: Problem<'tu>, source: SourceVectors) {
        if self.context.phase_four_text
            && self.context.configuration.compiler_identity() == CompilerIdentity::Gcc
        {
            return;
        }
        self.context.preprocessor_error(PreprocessorError {
            error_type:     PreprocessorErrorType::PragmaDiagnostic(problem),
            source_vectors: source,
        });
    }
}

/// Whether `name` is bcc's own warning option, which neither compiler lists.
fn is_bcc_warning(name: &str) -> bool {
    name == "constant-promotion"
}

/// What a specifier of `#pragma warning` does to a warning it lists.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum WarningSpecifier {
    /// `disable`.
    Disable,
    /// `error`.
    Error,
    /// `default`: back to the command line's setting.
    Default,
    /// A level, or `once`: reports it, as a warning. bcc does not model
    /// warning levels or `once`'s single report.
    Enable,
    /// `suppress`, which `cl` applies to the next line only. bcc does not
    /// model that, so it changes nothing.
    Suppress,
}

fn name_or_level(name: Option<&str>) -> Option<WarningSpecifier> {
    match name?.trim_end_matches('\0') {
        | "disable" => Some(WarningSpecifier::Disable),
        | "error" => Some(WarningSpecifier::Error),
        | "default" => Some(WarningSpecifier::Default),
        | "once" | "1" | "2" | "3" | "4" => Some(WarningSpecifier::Enable),
        | "suppress" => Some(WarningSpecifier::Suppress),
        | _ => None,
    }
}
