use std::ops::ControlFlow;

use super::super::{
    Expander,
    errors::{
        PreprocessorError,
        PreprocessorErrorType,
    },
    macro_expansion::MacroDefinition,
};
use crate::{
    configuration::Feature,
    translation_phases::{
        Context,
        TranslationPhase,
        preprocessor_tokenizer::{
            PreprocessorToken,
            PreprocessorTokenType,
            TokenSource,
        },
    },
    util::bump::ArenaVec,
};

impl Expander<'_, '_, '_, '_> {
    /// Defines an object-like or function-like macro.
    ///
    /// C99: the `# define` forms of §6.10 paragraph 1, p. 146; PDF p. 158,
    /// and §6.10.3 paragraphs 1-3 and 5-10, pp. 151-152; PDF pp. 163-164. A
    /// `(` with no whitespace before it (`lparen`) makes the macro
    /// function-like. A redefinition must match the old definition
    /// (paragraph 2), and a predefined name may not be redefined (§6.10.8
    /// paragraph 4, p. 161; PDF p. 173).
    ///
    /// Missing whitespace after an object-like macro's name (§6.10.3
    /// paragraph 3) and `__VA_ARGS__` outside a variadic macro's replacement
    /// list (paragraph 5) draw warnings and keep the definition. A duplicate
    /// parameter name (paragraph 6) or `##` at either end of the replacement
    /// list (§6.10.3.3 paragraph 1, p. 154; PDF p. 166) is an error that
    /// discards the definition, as GCC does, so its uses do not expand into
    /// further errors.
    pub(in crate::translation_phases::preprocessing) fn parse_define_directive(&mut self) {
        let Some(name) = self.expect_token_from_previous_phase::<true>(
            |_, t| t.kind.is_identifier(),
            |_, token| {
                ControlFlow::Break(PreprocessorError {
                    error_type:     PreprocessorErrorType::ExpectedIdentifierInDefineDirective(
                        token.kind,
                    ),
                    source_vectors: token.source_vectors,
                })
            },
            "parsing define directive",
        ) else {
            self.skip_until_newline();
            return;
        };
        let old_definition = self
            .state
            .macro_definitions
            .get(&name.identifier_id(self.context))
            .cloned();
        let tokenizer = self.tokenizer.clone();
        let mut old_tokenizer = match old_definition {
            | None => None,
            | Some(ref v) => match v {
                | MacroDefinition::FunctionLike { tokenizer, .. }
                | MacroDefinition::ObjectLike { tokenizer, .. } => Some(tokenizer.clone()),
                // Not one of the names C99 §6.10.8 predefines, so it may be
                // redefined, as in GCC.
                | MacroDefinition::BuiltIn
                    if implementation_macro(self.context.string_cache.at(name.contents)) =>
                    None,
                | MacroDefinition::BuiltIn => {
                    // C99 §6.10.8p4: predefined macro names cannot be
                    // redefined, so the built-in definition stays in effect.
                    self.context.preprocessor_error(PreprocessorError {
                        error_type:     PreprocessorErrorType::RedefinitionOfBuiltInMacro(
                            self.context
                                .diagnostic_text(self.context.string_cache.at(name.contents)),
                        ),
                        source_vectors: name.source_vectors,
                    });
                    if !super::language_features::overridable_gnu_builtin(
                        self.context.string_cache.at(name.contents),
                    ) {
                        self.skip_until_newline();
                        return;
                    }
                    None
                },
            },
        };
        // C99 §6.10.3p5: `__VA_ARGS__` is not a macro name.
        self.check_va_args_use(name);
        // A function-like definition requires '(' immediately after its name.
        let probe = self.tokenizer.next_item(self.context);
        match probe {
            | Some(PreprocessorToken {
                kind: PreprocessorTokenType::OpeningParenthesis,
                ..
            }) => {},
            // C99 §6.10.3p3: whitespace separates an object-like macro's name
            // from its replacement list. Without it the tokens are still the
            // replacement list, as in GCC and Clang.
            | Some(token)
                if !matches!(
                    token.kind,
                    PreprocessorTokenType::Whitespace | PreprocessorTokenType::Newline
                ) =>
                self.context.preprocessor_error(PreprocessorError {
                    error_type:     PreprocessorErrorType::MissingWhitespaceAfterMacroName(
                        self.context
                            .diagnostic_text(self.context.string_cache.at(name.contents)),
                    ),
                    source_vectors: token.source_vectors,
                }),
            | Some(_) => {},
            | None => self.context.preprocessor_error(PreprocessorError {
                error_type:     PreprocessorErrorType::UnexpectedEndOfInput(
                    "parsing macro definition",
                ),
                source_vectors: name.source_vectors,
            }),
        }
        // A definition that breaks a constraint whose meaning C99 leaves open
        // is discarded rather than guessed at.
        let mut is_valid = true;
        // Collected in the expansion arena; the definition keeps a copy.
        let mut argument_names = ArenaVec::new_in(self.scratch);
        let mut is_variadic = false;
        let mut variadic_alias = None;
        let is_function_like =
            probe.is_some_and(|token| token.kind == PreprocessorTokenType::OpeningParenthesis);
        let (body, first) = if is_function_like {
            if old_definition.as_ref().is_some_and(|d| match d {
                | MacroDefinition::ObjectLike { .. } => true,
                // Overridable GNU builtins have no source macro shape.
                | MacroDefinition::FunctionLike { .. } | MacroDefinition::BuiltIn => false,
            }) {
                self.context.preprocessor_error(PreprocessorError {
                    error_type:
                        PreprocessorErrorType::RedefinitionOfObjectLikeMacroAsFunctionLikeMacro(
                            self.context
                                .diagnostic_text(self.context.string_cache.at(name.contents)),
                            self.context.configuration.extension_policy(),
                        ),
                    source_vectors: name.source_vectors,
                });
                // Already diagnosed; comparing the lists would repeat it.
                old_tokenizer = None;
            }
            loop {
                let Some(name_or_ellipsis) = self.expect_token_from_previous_phase::<true>(
                    |_, t| {
                        t.kind.is_identifier()
                            || t.kind == PreprocessorTokenType::Ellipsis
                            || (t.kind == PreprocessorTokenType::ClosingParenthesis
                                && argument_names.is_empty())
                    },
                    |_, token| {
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
                    is_valid = false;
                    break;
                };
                if is_variadic {
                    is_valid = false;
                    self.context.preprocessor_error(PreprocessorError {
                        error_type:     PreprocessorErrorType::VariadicMacroMustBeLastParameter(
                            self.context
                                .diagnostic_text(self.context.string_cache.at(name.contents)),
                        ),
                        source_vectors: name.source_vectors,
                    });
                }
                if name_or_ellipsis.kind == PreprocessorTokenType::Ellipsis {
                    is_variadic = true;
                } else if name_or_ellipsis.kind.is_identifier() {
                    // C99 §6.10.3p5: `__VA_ARGS__` is not a parameter name.
                    self.check_va_args_use(name_or_ellipsis);
                    let parameter = name_or_ellipsis.identifier_id(self.context);
                    // C99 §6.10.3p6: each parameter is declared once.
                    if argument_names.contains(&parameter) {
                        self.context.preprocessor_error(PreprocessorError {
                            error_type:     PreprocessorErrorType::DuplicateMacroParameter(
                                self.context.diagnostic_text(
                                    self.context.string_cache.at(name_or_ellipsis.contents),
                                ),
                            ),
                            source_vectors: name_or_ellipsis.source_vectors,
                        });
                        is_valid = false;
                    }
                    argument_names.push(parameter);
                } else if name_or_ellipsis.kind == PreprocessorTokenType::ClosingParenthesis {
                    break;
                }
                let Some(comma_or_closing_parent) = self.expect_token_from_previous_phase::<true>(
                    |_, t| t.kind == PreprocessorTokenType::Comma || t.kind == PreprocessorTokenType::ClosingParenthesis || (t.kind == PreprocessorTokenType::Ellipsis && !is_variadic),
                    |_, token|
                        ControlFlow::Break(PreprocessorError {
                                error_type:     PreprocessorErrorType::ExpectedCommaOrClosingParenthesisInMacroDefinition(token.kind),
                                source_vectors: token.source_vectors,
                            },
                        )
                    ,
                    "parsing macro definition",
                ) else {is_valid = false; break;};
                if comma_or_closing_parent.kind == PreprocessorTokenType::Ellipsis {
                    variadic_alias = argument_names.pop();
                    is_variadic = true;
                    self.context.report_extension(
                        Feature::NamedVariadicMacros,
                        "named variadic macro",
                        name_or_ellipsis.source_vectors,
                    );
                    let Some(closing) = self.expect_token_from_previous_phase::<true>(
                        |_, t| t.kind == PreprocessorTokenType::ClosingParenthesis,
                        |this, token| {
                            ControlFlow::Break(PreprocessorError {
                                error_type:
                                    PreprocessorErrorType::VariadicMacroMustBeLastParameter(
                                        this.context.diagnostic_text(
                                            this.context.string_cache.at(name.contents),
                                        ),
                                    ),
                                source_vectors: token.source_vectors,
                            })
                        },
                        "parsing named variadic macro",
                    ) else {
                        is_valid = false;
                        break;
                    };
                    _ = closing;
                    break;
                }
                if comma_or_closing_parent.kind == PreprocessorTokenType::ClosingParenthesis {
                    break;
                }
            }
            (self.tokenizer.clone(), None)
        } else {
            if old_definition.as_ref().is_some_and(|d| match d {
                | MacroDefinition::FunctionLike { .. } => true,
                // Overridable GNU builtins have no source macro shape.
                | MacroDefinition::ObjectLike { .. } | MacroDefinition::BuiltIn => false,
            }) {
                self.context.preprocessor_error(PreprocessorError {
                    error_type:
                        PreprocessorErrorType::RedefinitionOfFunctionLikeMacroAsObjectLikeMacro(
                            self.context
                                .diagnostic_text(self.context.string_cache.at(name.contents)),
                            self.context.configuration.extension_policy(),
                        ),
                    source_vectors: name.source_vectors,
                });
                // Already diagnosed; comparing the lists would repeat it.
                old_tokenizer = None;
            }
            // The probe already read the replacement list's first token.
            (tokenizer, probe)
        };
        let (list_is_valid, lists_match) = self.read_replacement_list(
            first,
            is_variadic,
            is_variadic && variadic_alias.is_none(),
            old_tokenizer.as_mut(),
        );
        if !(is_valid && list_is_valid) {
            self.resume_at_line_start();
            return;
        }
        // C99 §6.10.3p2: a redefinition repeats the parameters and the
        // replacement list.
        let parameters_match = match &old_definition {
            | Some(MacroDefinition::FunctionLike {
                argument_names: old_argument_names,
                is_variadic: old_is_variadic,
                variadic_alias: old_alias,
                ..
            }) if is_function_like =>
                argument_names[..] == old_argument_names[..]
                    && is_variadic == *old_is_variadic
                    && variadic_alias == *old_alias,
            | _ => true,
        };
        // A change of form was already reported above; as a warning it is
        // not folded into this one, so it is not repeated.
        let form_changed = match &old_definition {
            | Some(MacroDefinition::ObjectLike { .. }) => is_function_like,
            | Some(MacroDefinition::FunctionLike { .. }) => !is_function_like,
            | _ => false,
        };
        if old_tokenizer.is_some() && !form_changed && !(parameters_match && lists_match) {
            self.context.preprocessor_error(PreprocessorError {
                error_type:     PreprocessorErrorType::MacroRedefinedWithDifferentDefinition(
                    self.context
                        .diagnostic_text(self.context.string_cache.at(name.contents)),
                    self.context.configuration.extension_policy(),
                ),
                source_vectors: name.source_vectors,
            });
        }
        if is_variadic {
            self.context.report_extension(
                Feature::VariadicMacros,
                "variadic macro",
                name.source_vectors,
            );
        }
        if !self.validate_variadic_body(body.clone(), is_variadic) {
            self.resume_at_line_start();
            return;
        }
        let tokenizer = self.state.lexed_files.persist(&body);
        let definition = if is_function_like {
            MacroDefinition::FunctionLike {
                tokenizer,
                argument_names: self.state.arena.alloc_slice_copy(&argument_names),
                is_variadic,
                variadic_alias,
            }
        } else {
            MacroDefinition::ObjectLike { tokenizer }
        };
        _ = self
            .state
            .macro_definitions
            .insert(name.identifier_id(self.context), definition);
        // Clang's implementation-defined pragma mark follows the name across
        // redefinitions (including C99 §6.10.3p2 identical definitions).
        // Only #undef ends it, as it ends the definition (§6.10.3.5p1).
        self.resume_at_line_start();
    }

    /// Ends a macro definition; a name that is not a macro is ignored.
    ///
    /// C99: §6.10.3.5 paragraph 2, p. 155; PDF p. 167. `#undef` of a
    /// predefined name, which §6.10.8 paragraph 4, p. 161; PDF p. 173
    /// forbids, is diagnosed and leaves the name defined, as a redefinition
    /// does.
    pub(in crate::translation_phases::preprocessing) fn parse_undef_directive(&mut self) {
        let Some(name) = self.expect_token_from_previous_phase::<true>(
            |_, t| t.kind.is_identifier(),
            |_, token| {
                ControlFlow::Break(PreprocessorError {
                    error_type:     PreprocessorErrorType::ExpectedIdentifierInUndefDirective(
                        token.kind,
                    ),
                    source_vectors: token.source_vectors,
                })
            },
            "parsing undef directive",
        ) else {
            self.skip_until_newline();
            return;
        };
        let name_id = name.identifier_id(self.context);
        _ = self.state.deprecated_macros.remove(&name_id);
        if matches!(
            self.state.macro_definitions.get(&name_id),
            Some(MacroDefinition::BuiltIn)
        ) && !implementation_macro(self.context.string_cache.at(name.contents))
        {
            // C99 §6.10.8p4: predefined macro names cannot be undefined, so
            // the built-in definition stays in effect.
            self.context.preprocessor_error(PreprocessorError {
                error_type:     PreprocessorErrorType::UndefinitionOfBuiltInMacro(
                    self.context
                        .diagnostic_text(self.context.string_cache.at(name.contents)),
                ),
                source_vectors: name.source_vectors,
            });
            if super::language_features::overridable_gnu_builtin(
                self.context.string_cache.at(name.contents),
            ) {
                _ = self.state.macro_definitions.remove(&name_id);
            }
        } else {
            _ = self.state.macro_definitions.remove(&name_id);
        }
        if self
            .expect_token_from_previous_phase::<true>(
                |_, t| t.kind == PreprocessorTokenType::Newline,
                |_, token| {
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
            self.skip_until_newline();
        }
        self.resume_at_line_start();
    }

    /// Reads a `#define` directive's replacement list through its new-line,
    /// starting with `first` when the caller has already read that token.
    /// Returns whether the list is valid where an invalid one discards the
    /// definition, and whether it matches the replacement list that `old`
    /// reads, if any.
    ///
    /// The list is read once, by the directive's own tokenizer, so the
    /// diagnostics recorded while lexing it are reported once.
    ///
    /// C99: `##` shall not begin or end a replacement list, §6.10.3.3
    /// paragraph 1, p. 154; PDF p. 166; `__VA_ARGS__` shall occur only in
    /// that of a variadic macro, §6.10.3 paragraph 5, p. 151; PDF p. 163; and
    /// lists match under paragraph 1, p. 151; PDF p. 163.
    fn read_replacement_list(
        &mut self,
        mut first: Option<PreprocessorToken>,
        is_variadic: bool,
        allows_va_args: bool,
        mut old: Option<&mut TokenSource<'_>>,
    ) -> (bool, bool) {
        let mut lists_match = true;
        let mut first_operand = None;
        let mut last_operand = None;
        let mut operands = 0usize;
        // C99 §6.10.3p1: whitespace separations match whatever their amount,
        // and §6.10.3p7 leaves leading and trailing whitespace out of the
        // list, so the lists are compared token by token, each with whether
        // whitespace separates it from the token before it.
        let mut separated = false;
        loop {
            let next = match first.take() {
                | Some(token) => Some(token),
                | None => self.tokenizer.next_item(self.context),
            };
            let Some(token) = next.filter(|token| token.kind != PreprocessorTokenType::Newline)
            else {
                if lists_match && let Some(old) = old.as_deref_mut() {
                    lists_match = next_replacement_token(self.context, old).is_none();
                }
                break;
            };
            if token.kind == PreprocessorTokenType::Whitespace {
                separated = true;
                continue;
            }
            if lists_match && let Some(old) = old.as_deref_mut() {
                lists_match = next_replacement_token(self.context, old).is_some_and(
                    |(old_token, old_separated)| {
                        (operands == 0 || old_separated == separated)
                            && same_replacement_token(self.context, &old_token, &token)
                    },
                );
            }
            separated = false;
            if !is_variadic
                || (!allows_va_args
                    && token.kind.is_identifier()
                    && self
                        .context
                        .string_cache
                        .at(token.identifier_id(self.context))
                        == "__VA_ARGS__")
            {
                self.check_va_args_use(token);
            }
            _ = first_operand.get_or_insert(token);
            last_operand = Some(token);
            operands += 1;
        }
        let mut is_valid = true;
        if let Some(token) = first_operand.filter(|t| t.kind == PreprocessorTokenType::HashHash) {
            self.context.preprocessor_error(PreprocessorError {
                error_type:     PreprocessorErrorType::MissingLeftHandSideOfHashHashOperator,
                source_vectors: token.source_vectors,
            });
            is_valid = false;
        }
        // A lone `##` is reported once, as the start of the list.
        if let Some(token) = last_operand.filter(|t| t.kind == PreprocessorTokenType::HashHash)
            && operands > 1
        {
            self.context.preprocessor_error(PreprocessorError {
                error_type:     PreprocessorErrorType::MissingRightHandSideOfHashHashOperator,
                source_vectors: token.source_vectors,
            });
            is_valid = false;
        }
        (is_valid, lists_match)
    }

    /// Diagnoses when `token`, read in a `#define` directive, is `__VA_ARGS__`
    /// or `__VA_OPT__`
    /// where it may not appear; the caller allows a variadic macro's
    /// replacement list.
    ///
    /// C99: §6.10.3 paragraph 5, p. 151; PDF p. 163. C23 adds
    /// `__VA_OPT__`, §6.10.5p5, p. 178; PDF p. 191.
    pub(in crate::translation_phases::preprocessing) fn check_va_args_use(
        &mut self,
        token: PreprocessorToken,
    ) {
        if token.kind.is_identifier() {
            let policy = self.context.configuration.extension_policy();
            let error_type = match self
                .context
                .string_cache
                .at(token.identifier_id(self.context))
            {
                | "__VA_ARGS__" => PreprocessorErrorType::VaArgsOutsideVariadicMacro(policy),
                | "__VA_OPT__" => PreprocessorErrorType::VaOptOutsideVariadicMacro(policy),
                | _ => return,
            };
            self.context.preprocessor_error(PreprocessorError {
                error_type,
                source_vectors: token.source_vectors,
            });
        }
    }
}

/// Whether `name` is a macro the implementation predefines beyond the names
/// of C99 §6.10.8, p. 160; PDF p. 172. §6.10.8 paragraph 4 does not protect
/// it, so `#undef` and `#define` apply as to any macro, as in GCC.
fn implementation_macro(name: &str) -> bool {
    name == "__STRICT_ANSI__"
}

/// Reads the next token of a macro definition's replacement list from
/// `source`, with whether whitespace separates it from the token before it.
/// Returns `None` at the line's end, so trailing whitespace is not part of
/// the list.
///
/// C99: §6.10.3 paragraphs 1 and 7, p. 151; PDF p. 163. A comment
/// is already part of the whitespace token that replaces it (§5.1.1.2
/// paragraph 1, phase 3, p. 10; PDF p. 22).
fn next_replacement_token(
    context: &mut Context<'_>,
    source: &mut TokenSource<'_>,
) -> Option<(PreprocessorToken, bool)> {
    let mut separated = false;
    loop {
        match source.next_item(context)? {
            | token if token.kind == PreprocessorTokenType::Whitespace => separated = true,
            | token if token.kind == PreprocessorTokenType::Newline => return None,
            | token => return Some((token, separated)),
        }
    }
}

/// Whether two replacement-list tokens are the same preprocessing token with
/// the same spelling.
///
/// C99: §6.10.3 paragraph 1, p. 151; PDF p. 163.
fn same_replacement_token(
    context: &Context<'_>,
    old: &PreprocessorToken,
    new: &PreprocessorToken,
) -> bool {
    old.kind == new.kind
        && context.string_cache.at(old.contents) == context.string_cache.at(new.contents)
}
