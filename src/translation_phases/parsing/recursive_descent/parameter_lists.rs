//! Recursive parameter-type-list and identifier-list grammar.
//!
//! Translation phase 7 syntax analysis (§5.1.1.2, p. 10; PDF p. 22) of the
//! contents of a function declarator's parentheses: `parameter-type-list`,
//! `parameter-list`, `parameter-declaration`, and `identifier-list` (C99:
//! §6.7.5 paragraph 1, p. 114; PDF p. 126; §A.2.2, pp. 413-414;
//! PDF pp. 425-426), with the function-declarator rules of §6.7.5.3,
//! pp. 118-121; PDF pp. 130-133.
//!
//! The list opens function prototype scope (§6.2.1 paragraph 4, p. 30;
//! PDF p. 42); every list keeps the bindings a later definition body
//! needs. An identifier that could be a typedef name or a parameter name is
//! taken as a typedef name (§6.7.5.3 paragraph 11, p. 119; PDF p. 131).
//! Left to semantic analysis: the storage-class constraint of paragraph 2
//! and the empty-list rule of paragraph 3, p. 118; PDF p. 130; the `(void)`
//! special case of paragraph 10 and the adjustments of paragraphs 7-8,
//! p. 119; PDF p. 131. Empty and identifier-list forms are obsolescent
//! (§6.11.6-§6.11.7, p. 163; PDF p. 175) but accepted. Parameters accumulate
//! in arena storage, so the 127-parameter minimum (§5.2.4.1, p. 21;
//! PDF p. 33) imposes no fixed ceiling.

use crate::{
    configuration::Feature,
    translation_phases::{
        SourceVectors,
        parsing::{
            Parser,
            declaration_syntax::{
                DeclarationSpecifiers,
                Declarator,
                DirectDeclarator,
                ParameterDeclaration,
            },
            errors::ParserErrorType,
            frames::{
                declaration_specifiers::DeclarationSpecifiersFrame,
                declarator::DeclaratorMode,
                expression_operators::is_operator,
                parameter_list::unknown_type_name_starts_prototype,
            },
            machine::{
                ParameterListResult,
                ParseFrame,
                ParseFrameKind,
                ParseValue,
                unexpected_return,
            },
            recovery::{
                SynchronizationKind,
                SynchronizationSet,
            },
            scope::{
                NameClass,
                ScopeKind,
                list_key,
            },
            syntax::Identifier,
        },
        preprocessing::{
            OperatorTokenType,
            TokenType,
        },
    },
    util::bump::ArenaVec,
};

impl<'tu, 'p> Parser<'_, 'tu, 'p> {
    /// Parses a prototype parameter list, or an identifier list when
    /// `allow_k_and_r`, through the suffix's closing parenthesis. Only a
    /// named declarator may take an identifier list; abstract ones take
    /// `parameter-type-list?` (§6.7.6 paragraph 1, p. 122; PDF p. 134).
    ///
    /// C99: parameter-type-list, parameter-list, parameter-declaration, and
    /// identifier-list are §6.7.5, p. 114; PDF p. 126; function declarator
    /// rules are §6.7.5.3, pp. 118-121; PDF pp. 130-133.
    #[inline(never)]
    pub(super) fn rd_parameter_list(
        &mut self,
        allow_k_and_r: bool,
    ) -> Option<ParameterListResult<'tu>> {
        let entry = self.rd_enter(ParseFrameKind::ParameterList)?;
        let mut list = ParameterListParts {
            parameters:              ArenaVec::new_in(self.arena),
            identifiers:             ArenaVec::new_in(self.arena),
            source_vectors:          ArenaVec::new_in(self.arena),
            is_variadic:             false,
            entry_scope_depth:       self.scopes.depth(),
            parameter_name_bindings: 0,
            suppression_entry:       self.pedantic_suppression,
        };
        self.pools.parameters.lend(&mut list.parameters);
        self.pools.identifiers.lend(&mut list.identifiers);
        self.pools.source_vectors.lend(&mut list.source_vectors);
        let result = self.rd_parameter_list_body(&mut list, allow_k_and_r);
        self.pools.parameters.reclaim(&mut list.parameters);
        self.pools.identifiers.reclaim(&mut list.identifiers);
        self.pools.source_vectors.reclaim(&mut list.source_vectors);
        self.study_leave(entry);
        result
    }

    /// Enters prototype scope and selects K&R versus prototype syntax. A
    /// visible typedef forces prototype syntax; only a non-typedef
    /// identifier can select the legacy identifier-list branch.
    ///
    /// C99: a visible typedef name selects a parameter declaration
    /// (§6.7.5.3 paragraph 11, p. 119; PDF p. 131).
    fn rd_parameter_list_body(
        &mut self,
        list: &mut ParameterListParts<'tu, 'p>,
        allow_k_and_r: bool,
    ) -> Option<ParameterListResult<'tu>> {
        self.scopes.enter_scope(ScopeKind::FunctionPrototype);
        let token = self.cursor.current();
        // C23: §6.7.7.1 paragraph 1, pp. 126-127; PDF pp. 139-140 permits an
        // ellipsis without a preceding parameter-list.
        if is_operator(token, OperatorTokenType::Ellipsis) {
            let token = token.expect("ellipsis exists");
            self.extension(
                Feature::C23Keywords,
                "variadic function without named parameters",
                token,
            );
            list.is_variadic = true;
            list.source_vectors.push(token.source_vectors);
            self.rd_consume();
            self.rd_close_after_ellipsis(list);
            return Some(self.rd_finish_prototype(list));
        }
        if allow_k_and_r
            && token.is_some_and(|token| {
                matches!(token.kind, TokenType::Identifier)
                    && !self.scopes.is_typedef(token.contents)
            })
            && !unknown_type_name_starts_prototype(self)
        {
            // C23: §6.7.7.1 paragraph 1, pp. 126-127; PDF pp. 139-140
            // removes identifier-list declarators. Retain the legacy shape
            // for recovery, but never present it as native C23 grammar.
            if !self
                .context
                .configuration
                .accepts(Feature::OldStyleFunctionDeclarators)
            {
                self.extension_diagnostic(
                    Feature::OldStyleFunctionDeclarators,
                    ParserErrorType::ExpectedIsoSyntax(
                        "a prototype parameter list in C23",
                        token.map(|x| x.kind),
                    ),
                    token,
                );
            }
            self.rd_identifier_list(list)?;
            // Both successful and recovered exits close prototype scope
            // before the enclosing declarator resumes.
            self.pedantic_suppression = list.suppression_entry;
            self.scopes.restore_depth(list.entry_scope_depth);
            let parameters = self.alloc_syntax_list(&mut list.identifiers);
            return Some(ParameterListResult {
                direct_declarator: DirectDeclarator::KAndRStyleFunction { parameters },
                source_vectors:    self.context.merge_vector_list(&list.source_vectors),
            });
        }
        self.rd_parameter_type_list(list)?;
        Some(self.rd_finish_prototype(list))
    }

    /// Parses an identifier list after its first identifier was seen.
    ///
    /// C99: identifier-list, §6.7.5 paragraph 1, p. 114; PDF p. 126.
    fn rd_identifier_list(&mut self, list: &mut ParameterListParts<'tu, 'p>) -> Option<()> {
        let mut diagnosed_mixed_parameter = false;
        'items: loop {
            let Some(token) = self.cursor.current() else {
                self.report(
                    ParserErrorType::ExpectedIdentifierInKAndRFunctionDeclaratorParameterList(None),
                    None,
                );
                return Some(());
            };
            // Once identifier-list syntax is selected, a declaration starter
            // cannot silently switch dialects mid-list (C99 §6.7.5p1: an
            // identifier-list holds only identifiers and commas). Diagnose
            // the list once, then parse each such parameter declaration
            // whole and discard it, so its declarator name is not mistaken
            // for the next identifier.
            if self.declaration_starter(token) {
                if !diagnosed_mixed_parameter {
                    self.report(
                        ParserErrorType::KAndRFunctionDeclaratorMixedWithModernDeclarator,
                        Some(token),
                    );
                }
                diagnosed_mixed_parameter = true;
                let specifiers = self.rd_parameter_specifiers()?;
                list.source_vectors.push(specifiers.source_vectors);
                let token = self.cursor.current();
                if !is_operator(token, OperatorTokenType::Comma)
                    && !is_operator(token, OperatorTokenType::ClosingParenthesis)
                    && let Some(declarator) = self.rd_declarator(DeclaratorMode::MaybeAbstract)?
                {
                    list.source_vectors.push(declarator.source_vectors);
                }
            } else if diagnosed_mixed_parameter
                && is_operator(Some(token), OperatorTokenType::Ellipsis)
            {
                // After a diagnosed prototype parameter, a trailing `...`
                // belongs to the same prototype-style list; reporting it
                // again would only repeat that diagnosis.
                list.source_vectors.push(token.source_vectors);
                self.rd_consume();
            } else if !matches!(token.kind, TokenType::Identifier) {
                // Typedef names are declaration starters, handled above.
                self.report(
                    ParserErrorType::ExpectedIdentifierInKAndRFunctionDeclaratorParameterList(
                        Some(token.kind),
                    ),
                    Some(token),
                );
                self.rd_recover_parameter(list, SynchronizationKind::KAndRParameter);
            } else {
                list.identifiers.push(Identifier::from_token(token));
                list.source_vectors.push(token.source_vectors);
                self.rd_consume();
            }
            loop {
                self.pedantic_suppression = list.suppression_entry;
                let token = self.cursor.current();
                if is_operator(token, OperatorTokenType::Comma) {
                    let token = token.expect("comma token exists");
                    list.source_vectors.push(token.source_vectors);
                    self.rd_consume();
                    continue 'items;
                }
                if is_operator(token, OperatorTokenType::ClosingParenthesis) {
                    let token = token.expect("closing-parenthesis token exists");
                    list.source_vectors.push(token.source_vectors);
                    self.rd_consume();
                    return Some(());
                }
                self.report(
                    ParserErrorType::ExpectedCommaOrClosingParenthesisInKAndRFunctionDeclaratorParameterList(
                        token.map(|token| token.kind),
                    ),
                    token,
                );
                if token.is_none()
                    || is_operator(token, OperatorTokenType::Semicolon)
                    || is_operator(token, OperatorTokenType::ClosingCurlyBrace)
                {
                    return Some(());
                }
                // Repair an omitted comma without discarding the next
                // parameter name: diagnose, then reprocess it as an item.
                if token.is_some_and(|token| {
                    matches!(token.kind, TokenType::Identifier)
                        && !self.scopes.is_typedef(token.contents)
                }) {
                    continue 'items;
                }
                // A declarator punctuator, as in `T *r`, means a parameter
                // declaration was written in the identifier list: skip the
                // rest of the item instead of stopping at its name and
                // reporting that again. Other stray tokens keep the next
                // name as a parameter.
                let declarator_follows = token.is_some_and(|token| {
                    matches!(
                        token.kind,
                        TokenType::Operator(
                            OperatorTokenType::Asterisk
                                | OperatorTokenType::OpeningSquareBracket
                                | OperatorTokenType::OpeningParenthesis
                        )
                    )
                });
                self.rd_recover_parameter(
                    list,
                    if declarator_follows {
                        SynchronizationKind::Parameter
                    } else {
                        SynchronizationKind::KAndRParameter
                    },
                );
            }
        }
    }

    /// Parses prototype parameters through the closing parenthesis. `, ...`
    /// terminates the list.
    ///
    /// C99: `parameter-list , ...` (§6.7.5 paragraph 1, p. 114; PDF p. 126;
    /// §6.7.5.3 paragraph 9, p. 119; PDF p. 131).
    fn rd_parameter_type_list(&mut self, list: &mut ParameterListParts<'tu, 'p>) -> Option<()> {
        'parameters: loop {
            self.rd_parameter_declaration(list)?;
            loop {
                self.pedantic_suppression = list.suppression_entry;
                let token = self.cursor.current();
                if is_operator(token, OperatorTokenType::ClosingParenthesis) {
                    let token = token.expect("closing-parenthesis token exists");
                    list.source_vectors.push(token.source_vectors);
                    self.rd_consume();
                    return Some(());
                }
                if is_operator(token, OperatorTokenType::Comma) {
                    let token = token.expect("comma token exists");
                    list.source_vectors.push(token.source_vectors);
                    self.rd_consume();
                    // A closing delimiter here means the comma had no
                    // following parameter, which gets its own diagnostic
                    // rather than a specifier cascade.
                    let token = self.cursor.current();
                    if is_operator(token, OperatorTokenType::Ellipsis) {
                        let token = token.expect("ellipsis token exists");
                        list.is_variadic = true;
                        list.source_vectors.push(token.source_vectors);
                        self.rd_consume();
                        self.rd_close_after_ellipsis(list);
                        return Some(());
                    }
                    if is_operator(token, OperatorTokenType::ClosingParenthesis) {
                        let token = token.expect("closing-parenthesis token exists");
                        self.report(
                            ParserErrorType::ExpectedParameterDeclarationAfterCommaInFunctionDeclarator(
                                Some(token.kind),
                            ),
                            Some(token),
                        );
                        list.source_vectors.push(token.source_vectors);
                        self.rd_consume();
                        return Some(());
                    }
                    if is_operator(token, OperatorTokenType::Semicolon)
                        || is_operator(token, OperatorTokenType::ClosingCurlyBrace)
                        || token.is_none()
                    {
                        self.report(
                            ParserErrorType::ExpectedParameterDeclarationAfterCommaInFunctionDeclarator(
                                token.map(|token| token.kind),
                            ),
                            token,
                        );
                        return Some(());
                    }
                    continue 'parameters;
                }
                self.report(
                    ParserErrorType::ExpectedCommaOrClosingParenthesisInFunctionDeclaratorParameterList(
                        token.map(|token| token.kind),
                    ),
                    token,
                );
                if token.is_none()
                    || is_operator(token, OperatorTokenType::Semicolon)
                    || is_operator(token, OperatorTokenType::ClosingCurlyBrace)
                {
                    return Some(());
                }
                // Preserve a plausible next parameter after an omitted comma
                // instead of consuming it during recovery.
                if token.is_some_and(|token| self.declaration_starter(token)) {
                    continue 'parameters;
                }
                self.rd_recover_parameter(list, SynchronizationKind::Parameter);
            }
        }
    }

    /// Parses one parameter declaration. Its name enters prototype scope as
    /// soon as its declarator completes and may hide typedefs in later
    /// entries (C99 §6.2.1p4 and p7).
    ///
    /// C99: parameter-declaration, §6.7.5 paragraph 1, p. 114; PDF p. 126.
    fn rd_parameter_declaration(&mut self, list: &mut ParameterListParts<'tu, 'p>) -> Option<()> {
        let specifiers = self.rd_parameter_specifiers()?;
        // parameter-declaration permits an absent abstract declarator, so a
        // separator can complete the parameter immediately.
        let token = self.cursor.current();
        if is_operator(token, OperatorTokenType::Comma)
            || is_operator(token, OperatorTokenType::ClosingParenthesis)
        {
            list.parameters.push(ParameterDeclaration {
                declaration_specifiers: specifiers,
                declarator:             None,
                source_vectors:         specifiers.source_vectors,
            });
            list.source_vectors.push(specifiers.source_vectors);
            return Some(());
        }
        let declarator = self.rd_declarator(DeclaratorMode::MaybeAbstract)?;
        if let Some(identifier) = declarator.and_then(Declarator::identifier)
            && self
                .scopes
                .publish_reporting_new(identifier.name, NameClass::Ordinary)
        {
            list.parameter_name_bindings += 1;
        }
        let source_vectors = match declarator {
            | Some(declarator) => self
                .context
                .merge_vectors(specifiers.source_vectors, declarator.source_vectors),
            | None => specifiers.source_vectors,
        };
        list.parameters.push(ParameterDeclaration {
            declaration_specifiers: specifiers,
            declarator,
            source_vectors,
        });
        list.source_vectors.push(source_vectors);
        Some(())
    }

    fn rd_parameter_specifiers(&mut self) -> Option<DeclarationSpecifiers<'tu>> {
        let value = self.rd_child(ParseFrame::DeclarationSpecifiers(
            DeclarationSpecifiersFrame::parameter(),
        ))?;
        let ParseValue::DeclarationSpecifiers(specifiers) = value else {
            unexpected_return!("parameter specifiers returned an unexpected value: {value:?}");
        };
        Some(specifiers)
    }

    /// Requires `)` immediately after `...`: the ellipsis is terminal in
    /// the grammar, so the owning `)` is the only legal continuation.
    fn rd_close_after_ellipsis(&mut self, list: &mut ParameterListParts<'tu, 'p>) {
        let mut can_unwind_variadic_recovery = false;
        loop {
            let token = self.cursor.current();
            if is_operator(token, OperatorTokenType::ClosingParenthesis) {
                let token = token.expect("closing-parenthesis token exists");
                list.source_vectors.push(token.source_vectors);
                self.rd_consume();
                return;
            }
            let Some(token) = token else {
                self.report(
                    ParserErrorType::UnexpectedEndOfVariadicFunctionDeclaratorParameterList,
                    None,
                );
                return;
            };
            self.report(
                ParserErrorType::ExpectedClosingParenthesisAfterEllipsisInFunctionDeclaratorParameterList(
                    token.kind,
                ),
                Some(token),
            );
            if is_operator(Some(token), OperatorTokenType::Semicolon)
                || is_operator(Some(token), OperatorTokenType::ClosingCurlyBrace)
                || can_unwind_variadic_recovery && self.declaration_starter(token)
            {
                return;
            }
            let follows_comma = is_operator(Some(token), OperatorTokenType::Comma);
            can_unwind_variadic_recovery = !follows_comma;
            self.rd_recover_parameter(
                list,
                if follows_comma {
                    SynchronizationKind::VariadicTrailingParameter
                } else {
                    SynchronizationKind::VariadicParameterList
                },
            );
        }
    }

    /// Leaves prototype scope and returns a prototype function suffix. Like
    /// the K&R exit, it never leaks prototype bindings into the enclosing
    /// scope. A list at any nesting depth may still belong to a function
    /// definition, whose body must see every name declared in its parameter
    /// declarations (C99 §6.2.1p4). Names a definition can rebuild from its
    /// parameter declarators need no copy; anything else, such as an
    /// enumerator declared in an array bound, is retained.
    fn rd_finish_prototype(
        &mut self,
        list: &mut ParameterListParts<'tu, 'p>,
    ) -> ParameterListResult<'tu> {
        self.pedantic_suppression = list.suppression_entry;
        let parameter_list = self.alloc_syntax_list(&mut list.parameters);
        if self.scopes.innermost_binding_count() > list.parameter_name_bindings {
            self.scopes
                .retain_innermost_bindings(list_key(&parameter_list));
        }
        self.scopes.restore_depth(list.entry_scope_depth);
        ParameterListResult {
            direct_declarator: DirectDeclarator::Function {
                parameter_list,
                is_variadic: list.is_variadic,
            },
            source_vectors:    self.context.merge_vector_list(&list.source_vectors),
        }
    }

    fn rd_recover_parameter(
        &mut self,
        list: &mut ParameterListParts<'tu, 'p>,
        kind: SynchronizationKind,
    ) {
        if let Some(recovered) = self.rd_recover(SynchronizationSet {
            kind,
            target: ParseFrameKind::ParameterList,
        }) {
            list.source_vectors.push(recovered);
        }
    }
}

/// Components of one parameter list accumulated before arena insertion.
struct ParameterListParts<'tu, 'p> {
    /// Prototype parameters.
    parameters:              ArenaVec<'p, ParameterDeclaration<'tu>>,
    /// K&R identifiers.
    identifiers:             ArenaVec<'p, Identifier>,
    /// Provenance accumulated across the entire parenthesized suffix.
    source_vectors:          ArenaVec<'p, SourceVectors>,
    /// Whether `...` terminated the prototype parameter list.
    is_variadic:             bool,
    /// Scope depth restored by every parameter-list exit.
    entry_scope_depth:       usize,
    /// Prototype-scope bindings introduced by parameter declarator names.
    /// Any further binding came from an enum specifier somewhere in the
    /// parameter declarations.
    parameter_name_bindings: usize,
    /// `__extension__` suppression depth on entry. Every separator and exit
    /// restores it, so a marker covers only the parameter it begins (GNU
    /// extension; C99 §5.1.1.3, p. 11; PDF p. 23).
    suppression_entry:       usize,
}
