//! Apply presumed line numbers and source-file names from line directives.
//!
//! C99: translation phase 4; line control, §6.10.4, p. 158; PDF p. 170.
//! Physical file identity remains available to include handling.

use std::{
    ops::ControlFlow,
    path::Path,
};

use crate::translation_phases::{
    preprocessing::{
        Expander,
        errors::{
            PreprocessorError,
            PreprocessorErrorType,
        },
        token::{
            StringTokenType,
            TokenType,
        },
    },
    preprocessor_tokenizer::{
        PreprocessorToken,
        PreprocessorTokenType,
    },
};

impl Expander<'_, '_, '_, '_> {
    /// Sets the presumed line number, and with a string literal the presumed
    /// file name, of the following line. The operands are macro-replaced
    /// first.
    ///
    /// C99: §6.10.4 paragraphs 1 and 3-5, p. 158; PDF p. 170. The line number
    /// must be a digit sequence from 1 to 2147483647; one outside that range
    /// is diagnosed and ignored. The string literal is decoded like any
    /// other; a wide or encoded one is diagnosed and its name ignored.
    /// C11: §6.10.4 paragraph 1, p. 173; PDF p. 191, retains the character
    /// string literal requirement for the newly available encoded literals.
    pub(in crate::translation_phases::preprocessing) fn parse_line_directive(&mut self) {
        let Some(token) = self.expect_token_without_rewind::<true>(
            |_, t| t.kind == PreprocessorTokenType::Number,
            |_, t| {
                ControlFlow::Break(PreprocessorError {
                    error_type:     PreprocessorErrorType::MissingNumberInLineDirective(t.kind),
                    source_vectors: t.source_vectors,
                })
            },
            "parsing line directive",
        ) else {
            if !self.current_is_newline {
                self.skip_and_expand_until_newline();
            }
            return;
        };
        let digits = self
            .context
            .string_cache
            .at(token.contents)
            .trim_end_matches('\0');
        if !digits.bytes().all(|b| b.is_ascii_digit()) {
            self.context.preprocessor_error(PreprocessorError {
                error_type:     PreprocessorErrorType::LineDirectiveIsNotASimpleDigitSequence,
                source_vectors: token.source_vectors,
            });
            self.skip_and_expand_until_newline();
            return;
        }
        let mut value = digits.bytes().try_fold(0u32, |value, digit| {
            value
                .checked_mul(10)?
                .checked_add(u32::from(digit - b'0'))
                .filter(|value| i32::try_from(*value).is_ok())
        });
        if value.is_none() {
            self.context.preprocessor_error(PreprocessorError {
                error_type:     PreprocessorErrorType::LineDirectiveNumberTooLarge(
                    self.context.diagnostic_text(digits),
                ),
                source_vectors: token.source_vectors,
            });
        } else if value == Some(0) {
            // C99 §6.10.4p3: zero is undefined; like a number that is too
            // large, it is diagnosed and ignored.
            self.context.preprocessor_error(PreprocessorError {
                error_type:     PreprocessorErrorType::LineDirectiveNumberZero(
                    self.context.diagnostic_text(digits),
                ),
                source_vectors: token.source_vectors,
            });
            value = None;
        }
        let name = self.expect_token_without_rewind::<true>(
            |_, t| {
                matches!(
                    t.kind,
                    PreprocessorTokenType::String
                        | PreprocessorTokenType::GeneratedString
                        | PreprocessorTokenType::Newline
                )
            },
            |_, t| {
                ControlFlow::Break(PreprocessorError {
                    error_type:     PreprocessorErrorType::MissingNewlineAfterLineDirective(t.kind),
                    source_vectors: t.source_vectors,
                })
            },
            "parsing line directive",
        );
        let Some(name) = name else {
            self.skip_and_expand_until_newline();
            return;
        };
        let mut filename = None;
        if let t @ PreprocessorToken {
            kind: PreprocessorTokenType::String | PreprocessorTokenType::GeneratedString,
            ..
        } = name
        {
            // Unlike include names, #line names use normal string-literal
            // decoding. Save the result until the directive is complete.
            if let Some(token) = self.map_preprocessor_token(t) {
                match token.kind {
                    | TokenType::String(StringTokenType::String(contents)) => {
                        filename = self
                            .context
                            .literal_text_in(self.scratch, contents, false)
                            .filter(|text| !text.contains('\0'));
                        if filename.is_none() {
                            self.context.preprocessor_error(PreprocessorError {
                                error_type:     PreprocessorErrorType::InvalidLineFilename,
                                source_vectors: token.source_vectors,
                            });
                        }
                    },
                    // C99 §6.10.4p1 requires a character string literal. The
                    // name is ignored; the line number still applies.
                    | TokenType::String(StringTokenType::WideString(_)) =>
                        self.context.preprocessor_error(PreprocessorError {
                            error_type:     PreprocessorErrorType::WideStringInLineDirective,
                            source_vectors: token.source_vectors,
                        }),
                    | TokenType::String(StringTokenType::EncodedString(_, encoding)) =>
                        self.context.preprocessor_error(PreprocessorError {
                            error_type:     PreprocessorErrorType::EncodedStringInLineDirective(
                                encoding.prefix(),
                            ),
                            source_vectors: token.source_vectors,
                        }),
                    | _ => {},
                }
            }
            if self
                .expect_token_without_rewind::<true>(
                    |_, t| t.kind == PreprocessorTokenType::Newline,
                    |_, t| {
                        ControlFlow::Break(PreprocessorError {
                            error_type:     PreprocessorErrorType::MissingNewlineAfterLineDirective(
                                t.kind,
                            ),
                            source_vectors: t.source_vectors,
                        })
                    },
                    "parsing line directive",
                )
                .is_none()
            {
                self.skip_and_expand_until_newline();
                return;
            }
        }
        // The newline ends any macro operand frames and returns us to the
        // containing source file. C99 §6.10.4p3 assigns the following line.
        if let Some(value) = value {
            self.set_line(value);
        }
        if let Some(filename) = filename {
            // Naming the physical file itself keeps its identity, and so its
            // quoted source text; any other name gets an identity of its own
            // whose system-header status still follows the physical file.
            let physical = self.physical_source_file_index();
            let filename = Path::new(filename);
            let source_file_index = if self.context.get_source_file(physical) == filename {
                physical
            } else {
                self.context.add_presumed_source_file(filename, physical)
            };
            self.set_source_file_index(source_file_index);
        }
    }
}
