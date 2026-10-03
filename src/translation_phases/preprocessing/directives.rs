//! Directive dispatch and the non-conditional directives.

use std::{
    ops::ControlFlow,
    path::{
        Path,
        PathBuf,
    },
};

use super::{
    Preprocessor,
    driver::{
        TokenizerFrame,
        TokenizerFrameType,
    },
    errors::{
        PreprocessorError,
        PreprocessorErrorType,
    },
    macro_expansion::MacroDefinition,
    token::{
        StringTokenType,
        TokenType,
    },
};
use crate::{
    translation_phases::{
        Context,
        SetPosition,
        SetSourceFileIndex,
        SourceVectors,
        StrExt,
        TokenString,
        TranslationPhase,
        preprocessor_tokenizer::{
            PreprocessorToken,
            PreprocessorTokenType,
            TokenSource,
        },
    },
    util::{
        read_to_string_lossy,
        shared::SharedString,
        string_cache::StringCacheId,
    },
};

/// Maximum live header depth, excluding the main source file. C99 §5.2.4.1
/// requires support for at least 15 nested included files.
const MAX_INCLUDE_NESTING: usize = 200;

/// Compares one position of two macro definitions under C99 §6.10.3p2: the
/// tokens must be spelled identically, while any two whitespace separations
/// are equivalent and a line end matches the end of input.
fn same_replacement_token(
    context: &Context,
    old: Option<&PreprocessorToken>,
    new: Option<&PreprocessorToken>,
) -> bool {
    let ends = |token: Option<&PreprocessorToken>| {
        token.is_none_or(|token| token.kind == PreprocessorTokenType::Newline)
    };
    match (old, new) {
        | _ if ends(old) && ends(new) => true,
        | (Some(old), Some(new)) =>
            old.kind == new.kind
                && (old.kind == PreprocessorTokenType::Whitespace
                    || context.string_cache.at(old.contents)
                        == context.string_cache.at(new.contents)),
        | _ => false,
    }
}

#[expect(
    clippy::needless_continue,
    reason = "Explicit continues make this tokenizer's nested control flow easier to audit."
)]
#[expect(
    clippy::cast_possible_truncation,
    reason = "Integer literal values are range-checked before narrowing."
)]
#[expect(
    clippy::while_let_loop,
    reason = "The macro-parameter loop has multiple semantic exit conditions."
)]
impl Preprocessor {
    pub(super) fn parse_directive(&mut self, context: &mut Context, token: PreprocessorToken) {
        if !self.last_was_newline {
            context.preprocessor_error(PreprocessorError {
                error_type:     PreprocessorErrorType::HashMustBeFirstCharacterOnLine,
                source_vectors: token.source_vectors,
            });
        }
        let Some(directive) = Self::next_ignore_whitespace(&mut self.tokenizer, context) else {
            return;
        };
        match directive.kind {
            // Null directive.
            | PreprocessorTokenType::Newline => {
                self.last_was_newline = true;
                self.current_is_newline = true;
                return;
            },
            // This is the general case. We handle it in the function body.
            // If token is defined, it'll be handled when we match on contents.
            | PreprocessorTokenType::Defined
            | PreprocessorTokenType::Identifier
            | PreprocessorTokenType::UniversalIdentifier => (),
            | _ => {
                context.preprocessor_error(PreprocessorError {
                    error_type:     PreprocessorErrorType::HashMustBeFollowedByIdentifier,
                    source_vectors: directive.source_vectors,
                });
                self.skip_until_newline(context);
                return;
            },
        }
        match context.string_cache.at(directive.contents) {
            | "if" => self.parse_if_directive(context, directive),
            | "ifdef" => self.parse_ifdef_directive(context, directive),
            | "ifndef" => self.parse_ifndef_directive(context, directive),
            | "elif" => self.parse_elif_directive(context, directive),
            | "else" => self.parse_else_directive(context, directive),
            | "endif" => self.parse_endif_directive(context, directive),
            | "include" => self.parse_include_directive(context, directive),
            | "define" => self.parse_define_directive(context, directive),
            | "undef" => self.parse_undef_directive(context, directive),
            | "line" => self.parse_line_directive(context, directive),
            | "error" => self.parse_error_directive(context, directive),
            | "pragma" => {
                if !self.parse_pragma_directive(context, directive) {
                    self.skip_until_newline(context);
                }
                self.last_was_newline = true;
                self.current_is_newline = true;
            },
            | _ => {
                context.preprocessor_error(PreprocessorError {
                    error_type:     PreprocessorErrorType::UnknownDirective,
                    source_vectors: directive.source_vectors,
                });
                self.skip_until_newline(context);
            },
        }
    }

    pub(super) fn prepare_pragma_operator_string(
        &self,
        context: &Context,
        string: StringCacheId,
    ) -> SharedString {
        _ = self;
        let string = context.string_cache.at(string);
        let mut ret = String::new();
        // Skip the leading quote.
        let mut index = 1;
        if string.char_at(0) == Some('L') {
            index += 1;
        }
        while let Some(c) = string.char_at(index) {
            match c {
                | '\\' => match string.char_at(index + 1) {
                    | Some('"') => {
                        ret.push('"');
                        index += 2;
                    },
                    | Some('\\') => {
                        ret.push('\\');
                        index += 2;
                    },
                    | _ => {
                        // Only escaped quotes and backslashes are removed by
                        // C99 §6.10.9p1. Preserve other escapes and progress.
                        ret.push('\\');
                        index += 1;
                    },
                },
                | _ => {
                    ret.push(c);
                    index += c.len_utf8();
                },
            }
        }
        // Pop the trailing quote.
        if ret.ends_with('"') {
            _ = ret.pop();
        }
        ret.push('\n');
        SharedString::from(ret)
    }

    /// Resolves an include name the way GCC and Clang do (C99 §6.10.2p2-3
    /// leave the places implementation-defined):
    ///
    /// 1. `"name"` first looks beside the file containing the directive (for
    ///    `--input`, whose name has no directory, that is the working
    ///    directory), then in each `--iquote` directory.
    /// 2. Both forms then search each `--isystem` directory, `CPATH`, and
    ///    `C_INCLUDE_PATH`.
    ///
    /// The process working directory is never searched implicitly, so the
    /// result depends on the source tree rather than where the compiler runs.
    ///
    /// `including_file` is the file containing the directive, captured before
    /// a macro-expanded operand can switch to its definition's tokenizer.
    fn find_header_from_path(
        &mut self,
        context: &mut Context,
        including_file: u32,
        include_token: PreprocessorToken,
        path: &Path,
        is_system_header: bool,
    ) -> Option<u32> {
        let mut searched = Vec::new();
        let header = if path.is_absolute() {
            path.is_file().then(|| path.to_owned())
        } else {
            let mut candidates = Vec::new();
            if !is_system_header {
                let including_file = context.get_source_file(including_file);
                candidates.push(
                    including_file
                        .parent()
                        .map(Path::to_path_buf)
                        .unwrap_or_default(),
                );
                candidates.extend(self.quote_include_directories.iter().cloned());
            }
            candidates.extend(self.system_include_directories.iter().cloned());
            let mut found = None;
            for directory in candidates {
                let candidate = directory.join(path);
                if candidate.is_file() {
                    found = Some(candidate);
                    break;
                }
                searched.push(directory);
            }
            found
        };
        let Some(header) = header else {
            context.preprocessor_error(PreprocessorError {
                error_type:     PreprocessorErrorType::HeaderNotFound {
                    name: path.to_string_lossy().into_owned(),
                    is_system_header,
                    searched,
                },
                source_vectors: include_token.source_vectors,
            });
            return None;
        };
        let source_file_index = context.intern_source_file(header.into_boxed_path());
        if self.once_set.contains(&source_file_index) {
            None
        } else {
            Some(source_file_index)
        }
    }

    fn parse_include_directive(&mut self, context: &mut Context, directive: PreprocessorToken) {
        let including_file = self.physical_source_file_index();
        context.set_is_tokenizing_include_string(true);
        let include_string =
            self.expect_token_without_rewind::<true>(
                context,
                |_, context, token| match token.kind {
                    | PreprocessorTokenType::AngleBracketString
                    | PreprocessorTokenType::IncludeString
                    | PreprocessorTokenType::String => true,
                    | _ if context.string_cache.at(token.contents).starts_with('<') => true,
                    | _ => false,
                },
                |_, _, token| {
                    ControlFlow::Break(PreprocessorError {
                        error_type:
                            PreprocessorErrorType::ExpectedIncludeStringOrAngleBracketString(
                                token.kind,
                            ),
                        source_vectors: token.source_vectors,
                    })
                },
                "parsing include directive",
            );
        context.set_is_tokenizing_include_string(false);
        let Some(include_string) = include_string else {
            if !self.current_is_newline {
                self.skip_and_expand_until_newline(context);
            }
            return;
        };
        let header_source_index = match include_string.kind {
            | PreprocessorTokenType::IncludeString | PreprocessorTokenType::String => {
                let contents = context
                    .string_cache
                    .at(include_string.contents)
                    .strip_circumfix('"', '"')
                    .expect("Include strings must be enclosed in double quotes.")
                    .to_token_string();
                let path = Path::new(contents.as_str());
                self.find_header_from_path(context, including_file, include_string, path, false)
            },
            | PreprocessorTokenType::AngleBracketString => {
                let contents = context
                    .string_cache
                    .at(include_string.contents)
                    .strip_circumfix('<', '>')
                    .expect("Angle-bracket strings must be enclosed in angle brackets.")
                    .to_token_string();
                let path = Path::new(contents.as_str());
                self.find_header_from_path(context, including_file, include_string, path, true)
            },
            | _ => {
                let mut contents = TokenString::new();
                contents.push_str(&context.string_cache.at(include_string.contents)[1..]);
                let start_index = Context::duplicate_source_vectors(
                    &mut context.source_vectors.0,
                    include_string.source_vectors,
                );
                loop {
                    match self.next_preprocessor_token::<false>(context) {
                        | Some(token) => {
                            if token.kind == PreprocessorTokenType::Newline {
                                break;
                            }
                            let token_contents = context.string_cache.at(token.contents);
                            _ = Context::duplicate_source_vectors(
                                &mut context.source_vectors.0,
                                token.source_vectors,
                            );
                            if let Some(idx) = token_contents.find('>') {
                                contents.push_str(&token_contents[..idx]);
                                break;
                            }
                            contents.push_str(context.string_cache.at(token.contents));
                        },
                        | None => {
                            context.preprocessor_error(PreprocessorError {
                                error_type:     PreprocessorErrorType::UnexpectedEndOfInput(
                                    "parsing include directive",
                                ),
                                source_vectors: directive.source_vectors,
                            });
                            break;
                        },
                    }
                }
                let path = Path::new(contents.as_str());
                let synthetic_token = PreprocessorToken {
                    source_vectors: SourceVectors::new(
                        start_index,
                        context.source_vectors.0.len() as u32,
                    ),
                    contents:       context.string_cache.intern(&contents),
                    kind:           PreprocessorTokenType::AngleBracketString,
                };
                self.find_header_from_path(context, including_file, synthetic_token, path, true)
            },
        };
        if !self.current_is_newline
            && self
                .expect_token::<true>(
                    context,
                    |_, _, token| token.kind == PreprocessorTokenType::Newline,
                    |_, _, token| {
                        ControlFlow::Break(PreprocessorError {
                            error_type:     PreprocessorErrorType::ExtraTokensAfterIncludeDirective,
                            source_vectors: token.source_vectors,
                        })
                    },
                    "parsing include directive",
                )
                .is_none()
        {
            self.skip_and_expand_until_newline(context);
        }
        let Some(header_source_index) = header_source_index else {
            return;
        };
        // The main source contributes one frame. Macro frames and headers
        // whose processing has finished do not consume the nesting limit.
        let source_depth = self
            .tokenizer_stack
            .iter()
            .filter(|frame| matches!(frame.frame_type, TokenizerFrameType::SourceFile { .. }))
            .count();
        if source_depth > MAX_INCLUDE_NESTING {
            context.preprocessor_error(PreprocessorError {
                error_type:     PreprocessorErrorType::IncludeNestingLimitExceeded(
                    MAX_INCLUDE_NESTING,
                ),
                source_vectors: directive.source_vectors,
            });
            return;
        }
        let header_path = context.get_source_file(header_source_index);
        let Ok(header_string) = read_to_string_lossy(header_path).map_err(|e| {
            context.preprocessor_error(PreprocessorError {
                error_type:     PreprocessorErrorType::HeaderFileInaccessible(e),
                source_vectors: directive.source_vectors,
            });
        }) else {
            return;
        };
        let header_string = SharedString::from(header_string);
        context.record_source_text(header_source_index, header_string.clone());
        let tokenizer = TokenSource::new(context, header_source_index, header_string);
        self.push_tokenizer_frame(
            context,
            TokenizerFrame {
                frame_type: TokenizerFrameType::SourceFile {
                    conditional_base:           self.open_conditionals.len(),
                    physical_source_file_index: header_source_index,
                },
                tokenizer,
            },
        );
        self.last_was_newline = true;
        self.current_is_newline = true;
    }

    fn parse_define_directive(&mut self, context: &mut Context, _directive: PreprocessorToken) {
        let Some(name) = self.expect_token_from_previous_phase::<true>(
            context,
            |_, _, t| t.kind.is_identifier(),
            |_, _, token| {
                ControlFlow::Break(PreprocessorError {
                    error_type:     PreprocessorErrorType::ExpectedIdentifierInDefineDirective(
                        token.kind,
                    ),
                    source_vectors: token.source_vectors,
                })
            },
            "parsing define directive",
        ) else {
            self.skip_until_newline(context);
            return;
        };
        let old_definition = self
            .macro_definitions
            .get(&name.identifier_id(context))
            .cloned();
        let tokenizer = self.tokenizer.clone();
        let old_tokenizer = match old_definition {
            | None => None,
            | Some(ref v) => match v {
                | MacroDefinition::FunctionLike { tokenizer, .. }
                | MacroDefinition::ObjectLike { tokenizer, .. } => Some(tokenizer.clone()),
                | MacroDefinition::BuiltIn => {
                    // C99 §6.10.8p4: predefined macro names cannot be
                    // redefined, so the built-in definition stays in effect.
                    context.preprocessor_error(PreprocessorError {
                        error_type:     PreprocessorErrorType::RedefinitionOfBuiltInMacro(
                            context.string_cache.at(name.contents).to_owned(),
                        ),
                        source_vectors: name.source_vectors,
                    });
                    self.skip_until_newline(context);
                    return;
                },
            },
        };
        // A function-like definition requires '(' immediately after its name.
        // The probe may consume the directive's newline when the replacement
        // list is empty; remember that so the next line is not skipped too.
        let mut line_ended = false;
        let opening_paren = match self.tokenizer.next_item(context) {
            | Some(
                token @ PreprocessorToken {
                    kind: PreprocessorTokenType::OpeningParenthesis,
                    ..
                },
            ) => Some(token),
            | Some(token) => {
                line_ended = token.kind == PreprocessorTokenType::Newline;
                None
            },
            | None => {
                line_ended = true;
                context.preprocessor_error(PreprocessorError {
                    error_type:     PreprocessorErrorType::UnexpectedEndOfInput(
                        "parsing macro definition",
                    ),
                    source_vectors: name.source_vectors,
                });
                None
            },
        };
        if opening_paren.is_some() {
            if old_definition.as_ref().is_some_and(|d| match d {
                | MacroDefinition::ObjectLike { .. } => true,
                | MacroDefinition::FunctionLike { .. } => false,
                | MacroDefinition::BuiltIn => {
                    unreachable!("The case where name is a built-in macro is handled above")
                },
            }) {
                context.preprocessor_error(PreprocessorError {
                    error_type:
                        PreprocessorErrorType::RedefinitionOfObjectLikeMacroAsFunctionLikeMacro(
                            context.string_cache.at(name.contents).to_owned(),
                        ),
                    source_vectors: name.source_vectors,
                });
            }
            let mut argument_names = Vec::new();
            let mut is_variadic = false;
            loop {
                let Some(name_or_ellipsis) = self.expect_token_from_previous_phase::<true>(
                    context,
                    |_, _, t| {
                        t.kind.is_identifier()
                            || t.kind == PreprocessorTokenType::Ellipsis
                            || (t.kind == PreprocessorTokenType::ClosingParenthesis
                                && argument_names.is_empty())
                    },
                    |_, _, token| {
                        ControlFlow::Break(PreprocessorError {
                            error_type:
                                PreprocessorErrorType::ExpectedIdentifierInMacroDefinition(
                                    token.kind,
                                ),
                            source_vectors: token.source_vectors,
                        })
                    },
                    "parsing macro definition",
                ) else {
                    break;
                };
                if is_variadic {
                    context.preprocessor_error(PreprocessorError {
                        error_type:     PreprocessorErrorType::VariadicMacroMustBeLastParameter(
                            context.string_cache.at(name.contents).to_owned(),
                        ),
                        source_vectors: name.source_vectors,
                    });
                }
                if name_or_ellipsis.kind == PreprocessorTokenType::Ellipsis {
                    is_variadic = true;
                } else if name_or_ellipsis.kind.is_identifier() {
                    argument_names.push(name_or_ellipsis.identifier_id(context));
                } else if name_or_ellipsis.kind == PreprocessorTokenType::ClosingParenthesis {
                    break;
                }
                let Some(comma_or_closing_parent) = self.expect_token_from_previous_phase::<true>(
                    context,
                    |_, _, t| t.kind == PreprocessorTokenType::Comma || t.kind == PreprocessorTokenType::ClosingParenthesis,
                    |_, _, token|
                        ControlFlow::Break(PreprocessorError {
                                error_type:     PreprocessorErrorType::ExpectedCommaOrClosingParenthesisInMacroDefinition(token.kind),
                                source_vectors: token.source_vectors,
                            },
                        )
                    ,
                    "parsing macro definition",
                ) else {break;};
                if comma_or_closing_parent.kind == PreprocessorTokenType::ClosingParenthesis {
                    break;
                }
            }
            let tokenizer = self.tokenizer.clone();
            drop(self.macro_definitions.insert(
                name.identifier_id(context),
                MacroDefinition::FunctionLike {
                    tokenizer,
                    argument_names: argument_names.into(),
                    is_variadic,
                },
            ));
        } else {
            if old_definition.as_ref().is_some_and(|d| match d {
                | MacroDefinition::ObjectLike { .. } => false,
                | MacroDefinition::FunctionLike { .. } => true,
                | MacroDefinition::BuiltIn => {
                    unreachable!("The case where name is a built-in macro is handled above")
                },
            }) {
                context.preprocessor_error(PreprocessorError {
                    error_type:
                        PreprocessorErrorType::RedefinitionOfFunctionLikeMacroAsObjectLikeMacro(
                            context.string_cache.at(name.contents).to_owned(),
                        ),
                    source_vectors: name.source_vectors,
                });
            }
            drop(self.macro_definitions.insert(
                name.identifier_id(context),
                MacroDefinition::ObjectLike {
                    tokenizer: tokenizer.clone(),
                },
            ));
        }
        let mut last = Option::<PreprocessorToken>::None;
        if let Some(mut old_tokenizer) = old_tokenizer {
            // Compare replacement lists body to body, and parameter lists
            // separately (C99 §6.10.3p2).
            let (mut new_tokenizer, parameters_match) = match (
                self.macro_definitions.get(&name.identifier_id(context)),
                &old_definition,
            ) {
                | (
                    Some(MacroDefinition::FunctionLike {
                        tokenizer,
                        argument_names,
                        is_variadic,
                    }),
                    Some(MacroDefinition::FunctionLike {
                        argument_names: old_argument_names,
                        is_variadic: old_is_variadic,
                        ..
                    }),
                ) => (
                    tokenizer.clone(),
                    argument_names == old_argument_names && is_variadic == old_is_variadic,
                ),
                | (
                    Some(
                        MacroDefinition::FunctionLike { tokenizer, .. }
                        | MacroDefinition::ObjectLike { tokenizer },
                    ),
                    _,
                ) => (tokenizer.clone(), true),
                | _ => (tokenizer, true),
            };
            let mut error_has_been_generated = false;
            if !parameters_match {
                context.preprocessor_error(PreprocessorError {
                    error_type:     PreprocessorErrorType::MacroRedefinedWithDifferentDefinition(
                        context.string_cache.at(name.contents).to_owned(),
                    ),
                    source_vectors: name.source_vectors,
                });
                error_has_been_generated = true;
            }
            loop {
                let old_next = old_tokenizer.next_item(context);
                let new_next = new_tokenizer.next_item(context);
                if let Some(t) = new_next.as_ref()
                    && t.kind == PreprocessorTokenType::HashHash
                    && last.is_none()
                {
                    context.preprocessor_error(PreprocessorError {
                        error_type:
                            PreprocessorErrorType::MissingLeftHandSideOfHashHashOperator,
                        source_vectors: t.source_vectors,
                    });
                }
                if !same_replacement_token(context, old_next.as_ref(), new_next.as_ref())
                    && !error_has_been_generated
                {
                    context.preprocessor_error(PreprocessorError {
                        error_type:
                            PreprocessorErrorType::MacroRedefinedWithDifferentDefinition(
                                context.string_cache.at(name.contents).to_owned(),
                            ),
                        source_vectors: name.source_vectors,
                    });
                    error_has_been_generated = true;
                }
                if !new_next
                    .as_ref()
                    .is_some_and(|t| t.kind != PreprocessorTokenType::Newline)
                {
                    if last
                        .as_ref()
                        .is_some_and(|t| t.kind == PreprocessorTokenType::HashHash)
                    {
                        context.preprocessor_error(PreprocessorError {
                            error_type:
                                PreprocessorErrorType::MissingRightHandSideOfHashHashOperator,
                            source_vectors: last.unwrap().source_vectors,
                        });
                    }
                    break;
                }
                last = new_next;
            }
        }
        if !line_ended {
            loop {
                match self.tokenizer.next_item(context) {
                    | Some(token) if token.kind == PreprocessorTokenType::Newline => break,
                    | None => break,
                    | Some(_) => continue,
                }
            }
        }
        self.last_was_newline = true;
        self.current_is_newline = true;
    }

    fn parse_undef_directive(&mut self, context: &mut Context, _directive: PreprocessorToken) {
        let Some(name) = self.expect_token_from_previous_phase::<true>(
            context,
            |_, _, t| t.kind.is_identifier(),
            |_, _, token| {
                ControlFlow::Break(PreprocessorError {
                    error_type:     PreprocessorErrorType::ExpectedIdentifierInUndefDirective(
                        token.kind,
                    ),
                    source_vectors: token.source_vectors,
                })
            },
            "parsing undef directive",
        ) else {
            self.skip_until_newline(context);
            return;
        };
        drop(self.macro_definitions.remove(&name.identifier_id(context)));
        if self
            .expect_token_from_previous_phase::<true>(
                context,
                |_, _, t| t.kind == PreprocessorTokenType::Newline,
                |_, _, token| {
                    ControlFlow::Break(PreprocessorError {
                        error_type:     PreprocessorErrorType::ExpectedNewlineAfterUndefDirective(
                            token.kind,
                        ),
                        source_vectors: token.source_vectors,
                    })
                },
                "parsing undef directive",
            )
            .is_none()
        {
            self.skip_until_newline(context);
        }
        self.last_was_newline = true;
        self.current_is_newline = true;
    }

    fn parse_line_directive(&mut self, context: &mut Context, _directive: PreprocessorToken) {
        let Some(token) = self.expect_token::<true>(
            context,
            |_, _, t| t.kind == PreprocessorTokenType::Number,
            |_, _, t| {
                ControlFlow::Break(PreprocessorError {
                    error_type:     PreprocessorErrorType::MissingNumberInLineDirective(t.kind),
                    source_vectors: t.source_vectors,
                })
            },
            "parsing line directive",
        ) else {
            self.skip_and_expand_until_newline(context);
            return;
        };
        let digits = context
            .string_cache
            .at(token.contents)
            .trim_end_matches('\0');
        if !digits.bytes().all(|b| b.is_ascii_digit()) {
            context.preprocessor_error(PreprocessorError {
                error_type:     PreprocessorErrorType::LineDirectiveIsNotASimpleDigitSequence,
                source_vectors: token.source_vectors,
            });
            self.skip_and_expand_until_newline(context);
            return;
        }
        let value = digits.bytes().try_fold(0u32, |value, digit| {
            value
                .checked_mul(10)?
                .checked_add(u32::from(digit - b'0'))
                .filter(|value| i32::try_from(*value).is_ok())
        });
        if value.is_none() {
            context.preprocessor_error(PreprocessorError {
                error_type:     PreprocessorErrorType::LineDirectiveNumberTooLarge(
                    digits.to_owned(),
                ),
                source_vectors: token.source_vectors,
            });
        }
        let name = self.expect_token::<true>(
            context,
            |_, _, t| {
                matches!(
                    t.kind,
                    PreprocessorTokenType::String
                        | PreprocessorTokenType::GeneratedString
                        | PreprocessorTokenType::Newline
                )
            },
            |_, _, t| {
                ControlFlow::Break(PreprocessorError {
                    error_type:     PreprocessorErrorType::MissingNewlineAfterLineDirective(t.kind),
                    source_vectors: t.source_vectors,
                })
            },
            "parsing line directive",
        );
        let Some(name) = name else {
            self.skip_and_expand_until_newline(context);
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
            if let Some(token) = self.map_preprocessor_token(context, t)
                && let TokenType::String(
                    StringTokenType::String(contents) | StringTokenType::WideString(contents),
                ) = token.kind
            {
                let wide = matches!(
                    token.kind,
                    TokenType::String(StringTokenType::WideString(_))
                );
                filename = context
                    .literal_text(contents, wide)
                    .filter(|text| !text.contains('\0'))
                    .map(|text| PathBuf::from(text).into_boxed_path());
                if filename.is_none() {
                    context.preprocessor_error(PreprocessorError {
                        error_type:     PreprocessorErrorType::InvalidLineFilename,
                        source_vectors: token.source_vectors,
                    });
                }
            }
            if self
                .expect_token::<true>(
                    context,
                    |_, _, t| t.kind == PreprocessorTokenType::Newline,
                    |_, _, t| {
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
                self.skip_and_expand_until_newline(context);
                return;
            }
        }
        // The newline ends any macro operand frames and returns us to the
        // containing source file. C99 §6.10.4p3 assigns the following line.
        if let Some(value) = value {
            self.set_line(context, value);
        }
        if let Some(filename) = filename {
            let source_file_index = context.intern_source_file(filename);
            self.set_source_file_index(context, source_file_index);
        }
    }

    fn parse_error_directive(&mut self, context: &mut Context, directive: PreprocessorToken) {
        let mut contents = String::new();
        // A directive ending at end of file is complete; the missing final
        // newline is diagnosed on its own.
        while let Some(token) = self.tokenizer.next_item(context) {
            if token.kind == PreprocessorTokenType::Newline {
                break;
            }
            contents.push_str(
                context
                    .string_cache
                    .at(token.contents)
                    .trim_end_matches('\0'),
            );
        }
        context.preprocessor_error(PreprocessorError {
            error_type:     PreprocessorErrorType::ErrorDirective(contents),
            source_vectors: directive.source_vectors,
        });
    }

    pub(super) fn parse_pragma_directive(
        &mut self,
        context: &mut Context,
        _directive: PreprocessorToken,
    ) -> bool {
        let mut consumed_newline = false;
        let mut completed_stdc = false;
        'base: loop {
            let Some(token) = Self::next_ignore_whitespace(&mut self.tokenizer, context) else {
                let source_vectors = self.current_location(context);
                context.preprocessor_error(PreprocessorError {
                    error_type: PreprocessorErrorType::UnexpectedEndOfInput(
                        "parsing pragma directive",
                    ),
                    source_vectors,
                });
                break 'base;
            };
            match token.kind {
                | PreprocessorTokenType::Whitespace => continue 'base,
                | PreprocessorTokenType::Newline => {
                    consumed_newline = true;
                    break 'base;
                },
                | PreprocessorTokenType::Identifier
                | PreprocessorTokenType::UniversalIdentifier => {
                    match context.string_cache.at(token.contents) {
                        | "once" => {
                            if !self.current_is_header(context) {
                                context.preprocessor_error(PreprocessorError {
                                    error_type:     PreprocessorErrorType::PragmaOnceInNonHeader,
                                    source_vectors: token.source_vectors,
                                });
                            }
                            _ = self.once_set.insert(self.physical_source_file_index());
                            match Self::next_ignore_whitespace(&mut self.tokenizer, context) {
                                | Some(token) if token.kind == PreprocessorTokenType::Newline => {
                                    consumed_newline = true;
                                    break 'base;
                                },
                                | Some(t) => {
                                    context.preprocessor_error(PreprocessorError {
                                        error_type:
                                            PreprocessorErrorType::ExtraTokensAfterPragmaOnce(
                                                t.kind,
                                            ),
                                        source_vectors: token.source_vectors,
                                    });
                                    break 'base;
                                },
                                | None => {
                                    let source_vectors = self.current_location(context);
                                    context.preprocessor_error(PreprocessorError {
                                        error_type: PreprocessorErrorType::UnexpectedEndOfInput(
                                            "parsing pragma directive",
                                        ),
                                        source_vectors,
                                    });
                                    break 'base;
                                },
                            }
                        },
                        | "STDC" => {
                            match Self::next_ignore_whitespace(&mut self.tokenizer, context) {
                                | Some(token) if token.kind == PreprocessorTokenType::Newline => {
                                    consumed_newline = true;
                                    context.preprocessor_error(PreprocessorError {
                                                error_type:     PreprocessorErrorType::STDCPragmaDirectiveWithoutArgument,
                                                source_vectors: token.source_vectors,
                                            },
                                        );
                                    break 'base;
                                },
                                | Some(token) => {
                                    let s = context.string_cache.at(token.contents);
                                    if !token.kind.is_identifier()
                                        || !matches!(
                                            s,
                                            "FP_CONTRACT" | "FENV_ACCESS" | "CX_LIMITED_RANGE"
                                        )
                                    {
                                        context.preprocessor_error(PreprocessorError {
                                            error_type:
                                                PreprocessorErrorType::UnknownPragmaSTDCArgument(
                                                    s.to_owned(),
                                                ),
                                            source_vectors: token.source_vectors,
                                        });
                                        break 'base;
                                    }
                                },
                                | None => {
                                    let source_vectors = self.current_location(context);
                                    context.preprocessor_error(PreprocessorError {
                                        error_type: PreprocessorErrorType::UnexpectedEndOfInput(
                                            "parsing pragma directive",
                                        ),
                                        source_vectors,
                                    });
                                    break 'base;
                                },
                            }

                            match Self::next_ignore_whitespace(&mut self.tokenizer, context) {
                                | Some(token) if token.kind == PreprocessorTokenType::Newline => {
                                    consumed_newline = true;
                                    context.preprocessor_error(PreprocessorError {
                                                error_type:     PreprocessorErrorType::STDCPragmaDirectiveWithoutOnOffSwitch,
                                                source_vectors: token.source_vectors,
                                            },
                                        );
                                    break 'base;
                                },
                                | Some(token) => {
                                    let s = context.string_cache.at(token.contents);
                                    if !token.kind.is_identifier()
                                        || !matches!(s, "ON" | "OFF" | "DEFAULT")
                                    {
                                        context.preprocessor_error(PreprocessorError {
                                                    error_type:     PreprocessorErrorType::MissingOnOffSwitchInSTDCPragma(s.to_owned()),
                                                    source_vectors: token.source_vectors,
                                                },
                                            );
                                        break 'base;
                                    }
                                },
                                | None => {
                                    let source_vectors = self.current_location(context);
                                    context.preprocessor_error(PreprocessorError {
                                        error_type: PreprocessorErrorType::UnexpectedEndOfInput(
                                            "parsing pragma directive",
                                        ),
                                        source_vectors,
                                    });
                                    break 'base;
                                },
                            }
                            completed_stdc = true;
                        },
                        | _ => {
                            if !completed_stdc {
                                // An unknown pragma is ignored as a whole.
                                // Preserve the caller's line state for _Pragma,
                                // whose payload uses a temporary tokenizer.
                                let line_state = (self.last_was_newline, self.current_is_newline);
                                self.skip_until_newline(context);
                                (self.last_was_newline, self.current_is_newline) = line_state;
                                consumed_newline = true;
                            }
                            break 'base;
                        },
                    }
                },
                | _ => {
                    context.preprocessor_error(PreprocessorError {
                        error_type:     PreprocessorErrorType::UnknownPragmaDirective,
                        source_vectors: token.source_vectors,
                    });
                    break 'base;
                },
            }
        }
        consumed_newline
    }
}
