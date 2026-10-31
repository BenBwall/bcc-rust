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
    /// must be a digit sequence from 1 to 2147483647. As in Clang, zero and
    /// numbers up to 4294967295 are applied and reported only under
    /// -pedantic; a larger number is an error and is ignored. The string
    /// literal is decoded like any other; a wide or encoded one is
    /// diagnosed and its name ignored. C11: §6.10.4 paragraph 1, p. 173;
    /// PDF p. 191, retains the character string literal requirement for the
    /// newly available encoded literals.
    pub(super) fn parse_line_directive(&mut self) {
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
        self.apply_line_directive(token, false);
    }

    /// A GNU line marker, `# 12 "file.c" 1 3`: the form GCC and Clang write
    /// in preprocessed output, which they read back as a `#line` directive
    /// whose name is followed by flags. `number` is the line number after
    /// the `#`. Of the flags (1 entering an include, 2 returning from one, 3
    /// a system header, 4 extern "C"), 3 makes the following lines a system
    /// header until the next line marker, as GCC and Clang read it, so
    /// their warnings are withheld; the others are ignored.
    ///
    /// GNU extension: GCC manual, "Preprocessor Output"
    /// <https://gcc.gnu.org/onlinedocs/cpp/Preprocessor-Output.html>, over
    /// the non-directive of C99 §6.10 paragraph 1, p. 145; PDF p. 157.
    pub(super) fn parse_line_marker(&mut self, number: PreprocessorToken) {
        self.apply_line_directive(number, true);
    }

    /// Checks and applies the line number `token` and the optional file
    /// name after it; a line `marker` may end with flags.
    fn apply_line_directive(&mut self, token: PreprocessorToken, marker: bool) {
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
        // Clang applies zero and numbers up to 4294967295, reporting them as
        // extensions only under -pedantic, and rejects larger numbers.
        let value = digits.bytes().try_fold(0u32, |value, digit| {
            value.checked_mul(10)?.checked_add(u32::from(digit - b'0'))
        });
        let pedantic = crate::translation_phases::policy_severity(
            self.context.configuration.extension_policy(),
            crate::translation_phases::DiagnosticPolicy::Extension,
        );
        let out_of_range = match value {
            | None => Some((true, Some(crate::translation_phases::ErrorSeverity::Error))),
            | Some(0) => Some((false, pedantic)),
            | Some(value) if i32::try_from(value).is_err() => Some((true, pedantic)),
            | Some(_) => None,
        };
        if let Some((too_large, Some(severity))) = out_of_range {
            let digits = self.context.diagnostic_text(digits);
            self.context.preprocessor_error(PreprocessorError {
                error_type:     if too_large {
                    PreprocessorErrorType::LineDirectiveNumberTooLarge(digits, severity)
                } else {
                    PreprocessorErrorType::LineDirectiveNumberZero(digits, severity)
                },
                source_vectors: token.source_vectors,
            });
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
        let mut system = false;
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
                        // Bytes that are not UTF-8 stay bytes, as their
                        // stand-ins, as GCC keeps them in `__FILE__`.
                        let (text, raw) =
                            self.context.literal_text_lossy_in(self.scratch, contents);
                        self.context.raw_source_bytes |= raw;
                        filename = Some(text).filter(|text| !text.contains('\0'));
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
            if marker {
                system = self.line_marker_flags();
            } else if self
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
            if marker {
                let at = self.context.first_source_vector(token.source_vectors).index;
                self.context.mark_system_region(physical, at, system);
            }
        }
    }

    /// Reads a line marker's flags through the end of its line and returns
    /// whether flag 3 is among them. As GCC's `read_flag` does, the flags
    /// must be one of 1 or 2, then 3, then 4 (only after 3), each optional
    /// and in that order; the first token out of that order is an error, and
    /// it and the flags after it are ignored.
    /// GNU extension: GCC preprocessor manual, "Preprocessor Output".
    fn line_marker_flags(&mut self) -> bool {
        let mut system = false;
        let mut last = 0;
        let mut failed = false;
        loop {
            let token = match Self::next_ignore_whitespace(&mut self.tokenizer, self.context) {
                | Some(PreprocessorToken {
                    kind: PreprocessorTokenType::Newline,
                    ..
                })
                | None => {
                    self.resume_at_line_start();
                    return system;
                },
                | Some(token) => token,
            };
            if failed {
                continue;
            }
            let text = self
                .context
                .string_cache
                .at(token.contents)
                .trim_end_matches('\0');
            let flag = match text {
                | "1" => 1,
                | "2" => 2,
                | "3" => 3,
                | "4" => 4,
                | _ => 0,
            };
            if token.kind != PreprocessorTokenType::Number
                || flag <= last
                || (flag == 2 && last != 0)
                || (flag == 4 && last != 3)
            {
                failed = true;
                let text = self.context.diagnostic_text(text);
                self.context.preprocessor_error(PreprocessorError {
                    error_type:     PreprocessorErrorType::InvalidLineMarkerFlag(text),
                    source_vectors: token.source_vectors,
                });
                continue;
            }
            system |= flag == 3;
            last = flag;
        }
    }
}
