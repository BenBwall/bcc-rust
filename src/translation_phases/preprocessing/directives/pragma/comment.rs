//! TCC's `#pragma comment(kind, "text")`, read under the TCC identity only.
//! TCC macro-replaces the operands, and hands an `option` string to
//! `tcc_set_options`, so that each option holds from there on in the
//! translation unit; `lib` names a library for the link, and any other kind
//! is ignored. A text of phase 4 writes the pragma back and carries none of
//! it out, as `tcc -E` keeps it unchanged.
//!
//! bcc applies the options of a pragma that change what the rest of the
//! unit means only where it can follow them from that point:
//! `-mms-bitfields` and `-mno-ms-bitfields` set the bit-field layout of the
//! records whose definitions end after the pragma, as TCC lays a record out
//! at its closing brace. Plain `char` keeps one signedness through a
//! translation unit, so `-f[no-]signed-char` and `-f[no-]unsigned-char` are
//! accepted only when they ask for the signedness in effect. Every other
//! option is refused, so that nothing TCC would change from there on
//! silently stays as it was.
//!
//! bcc adds no library to a link from a pragma. `lib` naming `m` asks for
//! what every link but a WebAssembly one already has, the C99 §7.12
//! functions (`-lm` on Linux, the C run-time library on Windows); any other
//! library draws a warning that the link must name it.
//!
//! TCC: `pragma_parse` in `tccpp.c`; `tcc_set_options` and `set_flag` in
//! `libtcc.c`.
//! C99: implementation-defined pragma behavior, §6.10.6 paragraph 1, p. 159;
//! PDF p. 171; plain `char`, §6.2.5 paragraph 15, p. 35; PDF p. 47;
//! bit-field allocation, §6.7.2.1 paragraph 10, p. 102; PDF p. 114.

use super::super::super::{
    Expander,
    errors::{
        PreprocessorError,
        PreprocessorErrorType,
    },
};
use crate::{
    translation_phases::{
        pragma_comment::LayoutPragma,
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

/// Why an option other than the bit-field layout and plain `char`'s
/// signedness is refused.
const NOT_APPLIED: &str = "bcc applies only -mms-bitfields, -mno-ms-bitfields and the plain char \
                           signedness already in effect from a pragma";

/// Why a change of plain `char`'s signedness is refused.
const CHAR_SIGNEDNESS: &str =
    "plain char keeps the signedness the command line gives it through a translation unit";

impl Expander<'_, '_, '_, '_> {
    /// Carries out the `#pragma comment` whose `comment` is `pragma`, up to
    /// and including its new-line.
    pub(in crate::translation_phases::preprocessing::directives) fn pragma_comment(
        &mut self,
        pragma: PreprocessorToken,
    ) {
        if self.state.as_text {
            let line_state = (self.last_was_newline, self.current_is_newline);
            self.skip_until_newline();
            (self.last_was_newline, self.current_is_newline) = line_state;
            return;
        }
        let mut tokens = ArenaVec::new_in(self.scratch);
        self.expand_pragma_line(pragma.source_vectors, |_, token| tokens.push(token));
        // TCC reads `(`, the kind, `,`, one narrow string literal and `)`,
        // and nothing after them.
        let operands = match *tokens.as_slice() {
            | [open, kind, comma, text, close, ..]
                if open.kind == PreprocessorTokenType::OpeningParenthesis
                    && comma.kind == PreprocessorTokenType::Comma
                    && close.kind == PreprocessorTokenType::ClosingParenthesis
                    && self.is_deprecation_message_literal(text) =>
                Some((kind, text)),
            | _ => None,
        };
        let Some((kind, text)) = operands else {
            self.context.preprocessor_error(PreprocessorError {
                error_type:     PreprocessorErrorType::MalformedTccPragma,
                source_vectors: pragma.source_vectors,
            });
            return;
        };
        let kind = kind
            .kind
            .is_identifier()
            .then(|| match self.context.string_cache.at(kind.contents) {
                | "lib" => Some(Kind::Library),
                | "option" => Some(Kind::Options),
                | _ => None,
            })
            .flatten();
        let mut options = ArenaString::new_in(self.scratch);
        self.append_deprecation_message(text, &mut options);
        if kind == Some(Kind::Library) {
            if options.as_str() != "m" || self.context.configuration.target().is_wasm() {
                let library = self.context.diagnostic_text(&options);
                self.context.preprocessor_error(PreprocessorError {
                    error_type:     PreprocessorErrorType::TccPragmaLibrary(library),
                    source_vectors: pragma.source_vectors,
                });
            }
            return;
        }
        if kind != Some(Kind::Options) {
            return;
        }
        // `tcc_set_options` splits the string at white space.
        for option in options.split_ascii_whitespace() {
            self.comment_option(pragma, option);
        }
    }

    /// Applies, or refuses, one option of `#pragma comment(option)`.
    fn comment_option(&mut self, pragma: PreprocessorToken, option: &str) {
        let layout = match option {
            | "-mms-bitfields" => Some(true),
            | "-mno-ms-bitfields" => Some(false),
            | _ => None,
        };
        if let Some(ms_bitfields) = layout {
            self.context.layout_pragmas.push(LayoutPragma {
                position: self.context.preprocessed_tokens,
                ms_bitfields,
            });
            return;
        }
        // TCC's `set_flag` reads `-fno-NAME` as NAME turned off.
        let signed = match option {
            | "-fsigned-char" | "-fno-unsigned-char" => Some(true),
            | "-funsigned-char" | "-fno-signed-char" => Some(false),
            | _ => None,
        };
        let reason = match signed {
            | Some(signed) if signed == self.context.configuration.layout().char_signed => return,
            | Some(_) => CHAR_SIGNEDNESS,
            | None => NOT_APPLIED,
        };
        let option = self.context.diagnostic_text(option);
        self.context.preprocessor_error(PreprocessorError {
            error_type:     PreprocessorErrorType::TccPragmaOption { option, reason },
            source_vectors: pragma.source_vectors,
        });
    }
}

/// The kinds of `#pragma comment` TCC carries out.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    /// `lib`: a library for the link.
    Library,
    /// `option`: options for `tcc_set_options`.
    Options,
}
