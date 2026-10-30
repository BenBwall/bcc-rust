//! Recursive statement and block grammar.
//!
//! Translation phase 7 syntax analysis (§5.1.1.2, p. 10; PDF p. 22) of
//! `statement` and its alternatives. C99: §6.8, pp. 131-139; PDF
//! pp. 143-151; §A.2.3, pp. 415-416; PDF pp. 427-428: labeled statements
//! §6.8.1, pp. 131-132; PDF pp. 143-144; compound statements and block items
//! §6.8.2, p. 132; PDF p. 144; expression and null statements §6.8.3,
//! p. 132; PDF p. 144; selection §6.8.4, p. 133; PDF p. 145; iteration
//! §6.8.5, p. 135; PDF p. 147; jumps §6.8.6, p. 136; PDF p. 148.
//!
//! Selection and iteration statements, and each of their substatements, are
//! blocks (§6.8.4 paragraph 3, p. 133; PDF p. 145; §6.8.5 paragraph 5,
//! p. 135; PDF p. 147), so they open implicit scopes; a compound statement
//! opens a block scope (§6.2.1 paragraph 4, p. 29; PDF p. 41). Declarations,
//! function definitions and extension statements call their own
//! productions. Only syntax and the duplicate-`default` constraint
//! (§6.8.4.2 paragraph 3, p. 134; PDF p. 146) are checked here; the other
//! statement constraints belong to semantic analysis.

use crate::{
    configuration::Feature,
    translation_phases::{
        SourceVectors,
        parsing::{
            Parser,
            errors::ParserErrorType,
            grammar::{
                ProductionKind,
                declarations::DeclarationContext,
                expression_operators::{
                    is_operator,
                    is_postfix_starter,
                },
                expressions::{
                    ExpressionBoundary,
                    ExpressionMode,
                    closer_follows_stray_run,
                },
                gnu::{
                    GnuKind,
                    GnuValue,
                },
                modern::{
                    ModernKind,
                    ModernValue,
                },
            },
            modern::SpecifierExtensionKind,
            recovery::{
                ExpressionTerminator,
                SynchronizationKind,
                SynchronizationSet,
            },
            scope::{
                NameClass,
                ScopeKind,
                SwitchScope,
            },
            syntax::{
                AttributedStatement,
                BlockItem,
                CaseRange,
                ConstantExpression,
                ConstantExpressionSlot,
                ExpressionSlot,
                ForInitializer,
                ForStatement,
                Identifier,
                SelectionHeader,
                Statement,
                StatementType,
            },
        },
        preprocessing::{
            KeywordTokenType,
            OperatorTokenType,
            Token,
            TokenType,
        },
    },
    util::bump::ArenaVec,
};

impl<'tu> Parser<'_, 'tu, '_> {
    /// C99: statements §6.8 paragraph 1, p. 131; PDF p. 143. Each recursive
    /// substatement restores precisely the scope depth it observed on entry.
    /// `leave_else` leaves a following `else` to an enclosing `if`
    /// (§6.8.4.1 paragraph 3, p. 134; PDF p. 146); `block_item` admits a
    /// C23 label without a statement at the end of a block.
    #[inline(never)]
    pub(super) fn parse_statement(
        &mut self,
        scope: Option<ScopeKind>,
        block_item: bool,
        leave_else: bool,
    ) -> Option<&'tu Statement<'tu>> {
        let active = self.enter_production(ProductionKind::Statement)?;
        let depth = self.scopes.depth();
        if let Some(scope) = scope {
            self.scopes.enter_scope(scope);
        }
        let mut statement = StatementParts {
            source: None,
            errors: self.hard_error_count,
            block_item,
            leave_else,
        };
        let result = self.parse_statement_body(&mut statement);
        if !self.resource_limit_reported {
            self.scopes.restore_depth(depth);
        }
        self.leave_production(active);
        result
    }

    fn parse_statement_body(&mut self, s: &mut StatementParts) -> Option<&'tu Statement<'tu>> {
        let kind = match self.parse_statement_kind(s)? {
            | Built::Kind(kind) => kind,
            | Built::Existing(statement) => return Some(statement),
        };
        let source_vectors = s.source.unwrap_or_else(|| self.missing_syntax_source());
        Some(self.alloc_syntax(Statement {
            kind,
            source_vectors,
            recovered: self.hard_error_count > s.errors,
        }))
    }

    fn parse_statement_kind(&mut self, s: &mut StatementParts) -> Option<Built<'tu>> {
        let token = self.cursor.current();
        if let Some(token) = token
            && let TokenType::Keyword(
                keyword @ (KeywordTokenType::MsAsm
                | KeywordTokenType::Try
                | KeywordTokenType::Leave),
            ) = token.kind
        {
            if keyword == KeywordTokenType::MsAsm
                && token.uses_ambiguous_asm(self.context.configuration)
                && self.cursor.following().is_some_and(|next| {
                    matches!(
                        next.kind,
                        TokenType::Operator(OperatorTokenType::OpeningParenthesis)
                            | TokenType::Keyword(
                                KeywordTokenType::Volatile
                                    | KeywordTokenType::Inline
                                    | KeywordTokenType::Goto
                            )
                    )
                })
            {
                return self.parse_gnu_statement(s, GnuKind::Asm { label: false });
            }
            let statement = self.parse_msvc(keyword, self.hard_error_count)?;
            return Some(Built::Existing(statement));
        }
        if let Some(token) = token {
            let kind = match token.kind {
                | TokenType::Keyword(KeywordTokenType::Asm) => Some(GnuKind::Asm { label: false }),
                | TokenType::Keyword(KeywordTokenType::LocalLabel) => Some(GnuKind::LocalLabels),
                | _ => None,
            };
            if let Some(kind) = kind {
                return self.parse_gnu_statement(s, kind);
            }
        }
        if self.attribute_starter(token) {
            let ModernValue::Attributes(attributes) =
                self.parse_modern(ModernKind::Attributes, None, self.hard_error_count)?
            else {
                unreachable!("statement attributes protocol")
            };
            s.source = Some(attributes.source_vectors);
            let statement = self.parse_statement(None, s.block_item, s.leave_else)?;
            self.merge_child_statement(s, statement);
            let node = self.alloc_syntax(AttributedStatement {
                attributes,
                statement,
            });
            return Some(Built::Kind(StatementType::Attributed(node)));
        }
        if is_operator(token, OperatorTokenType::OpeningCurlyBrace) {
            return Some(Built::Existing(self.parse_compound(false)?));
        }
        // C99 §6.8.3p1, p3: a lone `;` is the null statement.
        if is_operator(token, OperatorTokenType::Semicolon) {
            self.own_statement_token(s);
            return Some(Built::Kind(StatementType::Null));
        }
        // C99 §6.8.1p1: `identifier :` declares a label, which has function
        // scope (§6.2.1p3). Uniqueness (§6.8.1p3) is a semantic check.
        if let Some(token) = token
            && matches!(token.kind, TokenType::Identifier)
            && is_operator(self.cursor.following(), OperatorTokenType::Colon)
        {
            let identifier = Identifier::from_token(token);
            if let Some(labels) = self.label_scopes.innermost_mut() {
                _ = labels.definitions.insert(identifier.name);
            }
            self.own_statement_token(s);
            self.close_statement_part(s, OperatorTokenType::Colon, "identifier label");
            return self.parse_labeled(s, LabelPrefix::Identifier(identifier));
        }
        if let Some(token) = token
            && let TokenType::Keyword(keyword) = token.kind
        {
            match keyword {
                | KeywordTokenType::Return => {
                    self.own_statement_token(s);
                    return self.parse_return(s);
                },
                | KeywordTokenType::Break | KeywordTokenType::Continue => {
                    self.own_statement_token(s);
                    return Some(self.parse_simple_jump(s, keyword == KeywordTokenType::Break));
                },
                | KeywordTokenType::Goto => {
                    self.own_statement_token(s);
                    return self.parse_goto(s);
                },
                | KeywordTokenType::Case => {
                    self.own_statement_token(s);
                    return self.parse_case(s);
                },
                | KeywordTokenType::Default => {
                    // C99 §6.8.4.2p3: at most one `default` label per
                    // `switch`. The duplicate is the label itself, so the
                    // diagnostic belongs on its keyword rather than on
                    // whatever token follows it.
                    if self.switch_scopes.len() > self.switch_floor
                        && self
                            .switch_scopes
                            .last()
                            .is_some_and(|switch| switch.has_default)
                    {
                        self.report(ParserErrorType::DuplicateDefaultLabel, Some(token));
                    } else if self.switch_scopes.len() > self.switch_floor
                        && let Some(switch) = self.switch_scopes.last_mut()
                    {
                        switch.has_default = true;
                    }
                    self.own_statement_token(s);
                    self.close_statement_part(s, OperatorTokenType::Colon, "default label");
                    return self.parse_labeled(s, LabelPrefix::Default);
                },
                | KeywordTokenType::If => {
                    self.scopes.enter_scope(ScopeKind::ImplicitSelection);
                    self.own_statement_token(s);
                    return self.parse_header_statement(s, HeaderKind::If);
                },
                | KeywordTokenType::Switch => {
                    self.scopes.enter_scope(ScopeKind::ImplicitSelection);
                    self.switch_scopes.push(SwitchScope::default());
                    self.own_statement_token(s);
                    let kind = self.parse_header_statement(s, HeaderKind::Switch);
                    if kind.is_some() {
                        let _switch_scope = self
                            .switch_scopes
                            .pop()
                            .expect("switch statement owns its switch scope");
                    }
                    return kind;
                },
                | KeywordTokenType::While => {
                    self.scopes.enter_scope(ScopeKind::ImplicitIteration);
                    self.own_statement_token(s);
                    return self.parse_header_statement(s, HeaderKind::While);
                },
                | KeywordTokenType::Do => {
                    self.scopes.enter_scope(ScopeKind::ImplicitIteration);
                    self.own_statement_token(s);
                    return self.parse_do(s);
                },
                | KeywordTokenType::For => {
                    self.scopes.enter_scope(ScopeKind::ImplicitIteration);
                    self.own_statement_token(s);
                    return self.parse_for(s);
                },
                | KeywordTokenType::Sizeof | KeywordTokenType::Extension => {},
                | _ if self.declaration_starter(token) => {
                    self.report(
                        ParserErrorType::ExpectedStatement(Some(token.kind)),
                        Some(token),
                    );
                    return Some(self.recover_statement(s));
                },
                | _ => {},
            }
        }
        // A typedef name that continues into declaration specifiers or a
        // declarator starts a declaration, which C99 allows only as a block
        // item, never as a substatement (§6.8.2p1). Compound blocks route
        // such tokens to a declaration before a statement starts, so only
        // substatements reach this point.
        if let Some(token) = token
            && matches!(token.kind, TokenType::Identifier)
            && self.declaration_recovery_starts_here(token)
        {
            self.report(
                ParserErrorType::ExpectedStatement(Some(token.kind)),
                Some(token),
            );
            return Some(self.recover_statement(s));
        }
        // A closing delimiter or label colon cannot start any statement or
        // expression; skip just that token so the following statement parses
        // without a cascade.
        if let Some(token) = token
            && matches!(
                token.kind,
                TokenType::Operator(
                    OperatorTokenType::Colon
                        | OperatorTokenType::ClosingParenthesis
                        | OperatorTokenType::ClosingSquareBracket
                )
            )
        {
            self.report(
                ParserErrorType::ExpectedStatement(Some(token.kind)),
                Some(token),
            );
            self.own_statement_token(s);
            return Some(Built::Kind(StatementType::Null));
        }
        let stray_else = token
            .is_some_and(|token| matches!(token.kind, TokenType::Keyword(KeywordTokenType::Else)));
        if token.is_none() || is_operator(token, OperatorTokenType::ClosingCurlyBrace) || stray_else
        {
            self.report(
                ParserErrorType::ExpectedStatement(token.map(|token| token.kind)),
                token,
            );
            if stray_else && !s.leave_else {
                self.own_statement_token(s);
            }
            return Some(Built::Kind(StatementType::Null));
        }
        // C99 §6.8.3p1: `expression-statement`.
        let slot = self.parse_statement_expression_slot(
            s,
            ExpressionBoundary::Statement(ExpressionTerminator::Semicolon),
        )?;
        self.close_statement_part(s, OperatorTokenType::Semicolon, "expression statement");
        Some(Built::Kind(StatementType::Expression(slot)))
    }

    /// GNU extension: `__asm__` statements and `__label__` declarations.
    /// <https://gcc.gnu.org/onlinedocs/gcc/Extended-Asm.html>
    /// <https://gcc.gnu.org/onlinedocs/gcc/Local-Labels.html>
    fn parse_gnu_statement(&mut self, s: &mut StatementParts, kind: GnuKind) -> Option<Built<'tu>> {
        Some(Built::Kind(
            match self.parse_gnu(kind, self.hard_error_count)? {
                | GnuValue::Asm(asm) => {
                    s.source = Some(asm.source_vectors);
                    StatementType::Asm(asm)
                },
                | GnuValue::LocalLabels(labels, source) => {
                    s.source = Some(source);
                    StatementType::LocalLabels(labels)
                },
                | _ => unreachable!("GNU statement child protocol"),
            },
        ))
    }

    /// Skips a declaration-shaped substatement after its diagnostic and
    /// keeps a following `;` as a null statement.
    fn recover_statement(&mut self, s: &mut StatementParts) -> Built<'tu> {
        let recovered = self.recover(SynchronizationSet {
            kind:   SynchronizationKind::Statement,
            target: ProductionKind::Statement,
        });
        if let Some(recovered) = recovered {
            self.context.merge_into(&mut s.source, recovered);
        }
        if is_operator(self.cursor.current(), OperatorTokenType::Semicolon) {
            self.own_statement_token(s);
        }
        Built::Kind(StatementType::Null)
    }

    /// C99 §6.8.6p1: `return expression(opt) ;`, after the keyword.
    fn parse_return(&mut self, s: &mut StatementParts) -> Option<Built<'tu>> {
        let token = self.cursor.current();
        let identifier_continues_expression = token
            .is_some_and(|token| matches!(token.kind, TokenType::Identifier))
            && self.cursor.following().is_some_and(|following| {
                matches!(
                    following.kind,
                    TokenType::Operator(OperatorTokenType::Asterisk)
                ) || is_postfix_starter(following.kind)
            });
        if is_operator(token, OperatorTokenType::Semicolon) {
            self.own_statement_token(s);
            return Some(Built::Kind(StatementType::Return(None)));
        }
        let expression = if at_expression_boundary(token, ExpressionTerminator::Semicolon)
            || token.is_some_and(|token| {
                !identifier_continues_expression && self.declaration_recovery_starts_here(token)
            })
            || token.is_some_and(|token| matches!(token.kind, TokenType::Identifier))
                && is_operator(self.cursor.following(), OperatorTokenType::Colon)
        {
            None
        } else {
            Some(self.parse_statement_expression_slot(
                s,
                ExpressionBoundary::Statement(ExpressionTerminator::Semicolon),
            )?)
        };
        self.close_statement_part(s, OperatorTokenType::Semicolon, "return statement");
        Some(Built::Kind(StatementType::Return(expression)))
    }

    /// C99 §6.8.6p1: `break ;` and `continue ;`, after the keyword. C2y
    /// named loops add `break identifier ;`; without that `;` the identifier
    /// more likely starts the next statement after a `break` missing its own.
    fn parse_simple_jump(&mut self, s: &mut StatementParts, is_break: bool) -> Built<'tu> {
        if let Some(token) = self.cursor.current()
            && matches!(token.kind, TokenType::Identifier)
            && is_operator(self.cursor.following(), OperatorTokenType::Semicolon)
        {
            self.extension(Feature::NamedLoops, "named loop control", token);
            self.own_statement_token(s);
            self.close_statement_part(s, OperatorTokenType::Semicolon, "named jump statement");
            let identifier = Identifier::from_token(token);
            return Built::Kind(if is_break {
                StatementType::NamedBreak(identifier)
            } else {
                StatementType::NamedContinue(identifier)
            });
        }
        self.close_statement_part(s, OperatorTokenType::Semicolon, "jump statement");
        Built::Kind(if is_break {
            StatementType::Break
        } else {
            StatementType::Continue
        })
    }

    /// C99 §6.8.6p1: `goto identifier ;`, after the keyword, plus the GNU
    /// computed `goto * expression ;`. The target is checked against the
    /// function's labels later (§6.8.6.1p1).
    fn parse_goto(&mut self, s: &mut StatementParts) -> Option<Built<'tu>> {
        let token = self.cursor.current();
        if let Some(token) = token
            && is_operator(Some(token), OperatorTokenType::Asterisk)
        {
            self.extension(Feature::LabelsAsValues, "computed goto", token);
            self.own_statement_token(s);
            let expression = self
                .parse_expression(
                    ExpressionMode::Expression,
                    ExpressionBoundary::Statement(ExpressionTerminator::Semicolon),
                    ExpressionBoundary::Statement(ExpressionTerminator::Semicolon),
                )?
                .expression;
            self.context
                .merge_into(&mut s.source, expression.source_vectors);
            self.close_statement_part(s, OperatorTokenType::Semicolon, "computed goto");
            return Some(Built::Kind(StatementType::ComputedGoto(
                ExpressionSlot::Parsed(expression),
            )));
        }
        let identifier = if let Some(token) =
            token.filter(|token| matches!(token.kind, TokenType::Identifier))
        {
            let identifier = Identifier::from_token(token);
            if let Some(labels) = self.label_scopes.innermost_mut() {
                _ = labels.references.insert(identifier.name);
            }
            self.own_statement_token(s);
            identifier
        } else {
            self.report(
                ParserErrorType::ExpectedGotoLabel(token.map(|token| token.kind)),
                token,
            );
            Identifier::new(
                self.context.string_cache.intern("<missing-label>"),
                self.missing_syntax_source(),
            )
        };
        self.close_statement_part(s, OperatorTokenType::Semicolon, "goto statement");
        Some(Built::Kind(StatementType::Goto(identifier)))
    }

    /// C99 §6.8.1p1: `case constant-expression :`, after the keyword, plus
    /// the C2y/GNU case range. The integer-constant requirement (§6.8.4.2p3)
    /// is semantic.
    fn parse_case(&mut self, s: &mut StatementParts) -> Option<Built<'tu>> {
        if at_expression_boundary(self.cursor.current(), ExpressionTerminator::Colon) {
            let token = self.cursor.current();
            self.report(
                ParserErrorType::ExpectedStatementExpression(
                    "case label",
                    token.map(|token| token.kind),
                ),
                token,
            );
            let missing = ConstantExpressionSlot::Missing(self.missing_syntax_source());
            return self.parse_case_colon(s, missing);
        }
        let lower = self.parse_case_value(s)?;
        let Some(ellipsis) = self
            .cursor
            .current()
            .filter(|token| is_operator(Some(*token), OperatorTokenType::Ellipsis))
        else {
            return self.parse_case_colon(s, lower);
        };
        // C2y N3370 extends C99 §6.8.1p1, p. 131; PDF p. 143. Earlier modes
        // require GNU support; keep the range for recovery.
        if self.context.configuration.accepts(Feature::CaseRanges) {
            self.extension(Feature::CaseRanges, "case range", ellipsis);
        } else {
            self.report(
                ParserErrorType::ExpectedIsoSyntax(
                    "a single case value in this language mode",
                    Some(ellipsis.kind),
                ),
                Some(ellipsis),
            );
        }
        self.own_statement_token(s);
        let upper = self.parse_case_value(s)?;
        self.close_statement_part(s, OperatorTokenType::Colon, "case range");
        self.parse_labeled(s, LabelPrefix::Range(lower, upper))
    }

    fn parse_case_value(&mut self, s: &mut StatementParts) -> Option<ConstantExpressionSlot<'tu>> {
        let expression = self
            .parse_expression(
                ExpressionMode::ConstantExpression,
                ExpressionBoundary::Statement(ExpressionTerminator::Colon),
                ExpressionBoundary::Statement(ExpressionTerminator::Colon),
            )?
            .expression;
        self.merge_statement_slot(s, expression.source_vectors);
        Some(ConstantExpressionSlot::Parsed(ConstantExpression(
            expression,
        )))
    }

    fn parse_case_colon(
        &mut self,
        s: &mut StatementParts,
        value: ConstantExpressionSlot<'tu>,
    ) -> Option<Built<'tu>> {
        loop {
            let token = self.cursor.current();
            if is_operator(token, OperatorTokenType::Semicolon)
                && is_operator(self.cursor.following(), OperatorTokenType::Colon)
            {
                self.report_missing_operator(OperatorTokenType::Colon, "case label");
                self.own_statement_token(s);
                continue;
            }
            // A leftover run such as the `, 3` of `case 2, 3:` is skipped to
            // the real `:` instead of starting the labeled statement.
            if token.is_some()
                && !is_operator(token, OperatorTokenType::Colon)
                && closer_follows_stray_run(
                    self,
                    |token| matches!(token, TokenType::Operator(OperatorTokenType::Colon)),
                    true,
                )
                .is_some()
            {
                self.report_missing_operator(OperatorTokenType::Colon, "case label");
                let recovered = self.recover(SynchronizationSet {
                    kind:   SynchronizationKind::StatementExpression(ExpressionTerminator::Colon),
                    target: ProductionKind::Statement,
                });
                if let Some(recovered) = recovered {
                    self.context.merge_into(&mut s.source, recovered);
                }
                continue;
            }
            self.close_statement_part(s, OperatorTokenType::Colon, "case label");
            return self.parse_labeled(s, LabelPrefix::Case(value));
        }
    }

    /// The statement after a label prefix. C23 also admits a declaration or
    /// the end of a block.
    ///
    /// C99: `labeled-statement`, §6.8.1 paragraph 1, p. 131; PDF p. 143.
    fn parse_labeled(
        &mut self,
        s: &mut StatementParts,
        prefix: LabelPrefix<'tu>,
    ) -> Option<Built<'tu>> {
        let token = self.cursor.current();
        if is_operator(token, OperatorTokenType::ClosingCurlyBrace) {
            if !s.block_item {
                self.report(
                    ParserErrorType::ExpectedIsoSyntax(
                        "a statement after a label outside a compound block",
                        token.map(|x| x.kind),
                    ),
                    token,
                );
            }
            self.extension(
                Feature::C23Keywords,
                "label at end of compound statement",
                token.expect("closing brace exists"),
            );
            let child = self.alloc_syntax(Statement {
                kind:           StatementType::Null,
                source_vectors: s.source.unwrap_or_default(),
                recovered:      false,
            });
            return Some(Built::Kind(self.parse_label(prefix, child)));
        }
        // C99 §6.8.1p1: `identifier :` is another label even when the
        // identifier names a typedef; labels have their own name space
        // (§6.2.3p1).
        let is_label = token.is_some_and(|x| matches!(x.kind, TokenType::Identifier))
            && is_operator(self.cursor.following(), OperatorTokenType::Colon);
        let child = if !is_label
            && token.is_some_and(|x| self.declaration_starter(x))
            && self.extension_precedes_declaration()
            && (!self.attribute_starter(token) || self.attributes_precede_declaration())
        {
            if !s.block_item {
                self.report(
                    ParserErrorType::ExpectedIsoSyntax(
                        "a statement after a label outside a compound block",
                        token.map(|x| x.kind),
                    ),
                    token,
                );
            }
            self.extension(
                Feature::C23Keywords,
                "label before declaration",
                token.expect("declaration exists"),
            );
            let declaration = self.parse_block_declaration(DeclarationContext::Block)?;
            self.alloc_syntax(Statement {
                kind:           StatementType::Declaration(declaration),
                source_vectors: declaration.source_vectors,
                recovered:      declaration.recovered,
            })
        } else {
            self.parse_statement(None, s.block_item, s.leave_else)?
        };
        self.merge_child_statement(s, child);
        Some(Built::Kind(self.parse_label(prefix, child)))
    }

    fn parse_label(
        &mut self,
        prefix: LabelPrefix<'tu>,
        statement: &'tu Statement<'tu>,
    ) -> StatementType<'tu> {
        match prefix {
            | LabelPrefix::Identifier(identifier) => StatementType::Label(identifier, statement),
            | LabelPrefix::Case(expression) => StatementType::Case(expression, statement),
            | LabelPrefix::Default => StatementType::Default(statement),
            | LabelPrefix::Range(lower, upper) =>
                StatementType::CaseRange(self.alloc_syntax(CaseRange {
                    lower,
                    upper,
                    statement,
                })),
        }
    }

    /// `if`, `switch` and `while`: `( expression )` and one substatement,
    /// after the keyword. C23 admits a declaration in a selection header.
    ///
    /// C99: §6.8.4, p. 133; PDF p. 145; §6.8.5, p. 135; PDF p. 147.
    fn parse_header_statement(
        &mut self,
        s: &mut StatementParts,
        kind: HeaderKind,
    ) -> Option<Built<'tu>> {
        if is_operator(self.cursor.current(), OperatorTokenType::OpeningParenthesis) {
            self.own_statement_token(s);
        } else {
            self.report_missing_operator(OperatorTokenType::OpeningParenthesis, kind.name());
        }
        let token = self.cursor.current();
        let expression = if matches!(kind, HeaderKind::If | HeaderKind::Switch)
            && token.is_some_and(|x| self.declaration_starter(x))
            && self.extension_precedes_declaration()
        {
            self.extension(
                Feature::IfSwitchDeclarations,
                "selection declaration",
                token.expect("declaration exists"),
            );
            self.parse_selection_header(s)?
        } else if at_expression_boundary(token, ExpressionTerminator::ClosingParenthesis) {
            self.report(
                ParserErrorType::ExpectedStatementExpression(
                    kind.name(),
                    token.map(|token| token.kind),
                ),
                token,
            );
            ExpressionSlot::Missing(self.missing_syntax_source())
        } else {
            self.parse_statement_expression_slot(s, ExpressionBoundary::ClosingParenthesis)?
        };
        self.parse_header_closing(s, kind.name());
        let body = self.parse_statement(
            Some(kind.scope_kind()),
            false,
            kind == HeaderKind::If || s.leave_else,
        )?;
        self.merge_child_statement(s, body);
        Some(Built::Kind(match kind {
            | HeaderKind::If => {
                // C99 §6.8.4.1p3: `else` binds to the nearest `if`.
                let else_statement = if self.cursor.current().is_some_and(|token| {
                    matches!(token.kind, TokenType::Keyword(KeywordTokenType::Else))
                }) {
                    self.own_statement_token(s);
                    let otherwise = self.parse_statement(
                        Some(ScopeKind::ImplicitSelection),
                        false,
                        s.leave_else,
                    )?;
                    self.merge_child_statement(s, otherwise);
                    Some(otherwise)
                } else {
                    None
                };
                StatementType::If {
                    condition_expression: expression,
                    then_statement: body,
                    else_statement,
                }
            },
            | HeaderKind::Switch => StatementType::Switch {
                condition_expression: expression,
                body_statement:       body,
            },
            | HeaderKind::While => StatementType::While {
                condition_expression: expression,
                body_statement:       body,
            },
        }))
    }

    /// C23: `if ( declaration expression(opt) )` and the same for `switch`.
    fn parse_selection_header(&mut self, s: &mut StatementParts) -> Option<ExpressionSlot<'tu>> {
        let declaration = self.parse_block_declaration(DeclarationContext::SelectionHeader)?;
        s.source = Some(
            self.context
                .merge_vectors(s.source.unwrap_or_default(), declaration.source_vectors),
        );
        let token = self.cursor.current();
        let expression = if is_operator(token, OperatorTokenType::ClosingParenthesis)
            && !is_operator(self.cursor.previous, OperatorTokenType::Semicolon)
        {
            if !matches!(declaration.init_declarators.as_slice(), [init] if init.initializer.is_some())
            {
                self.report(
                    ParserErrorType::ExpectedIsoSyntax(
                        "a single initialized declaration in selection header",
                        token.map(|x| x.kind),
                    ),
                    token,
                );
            }
            None
        } else {
            Some(self.parse_statement_expression_slot(s, ExpressionBoundary::ClosingParenthesis)?)
        };
        let header = self.alloc_syntax(SelectionHeader {
            declaration,
            expression,
        });
        Some(ExpressionSlot::Selection(header))
    }

    /// The `)` closing a statement header. A `;` directly before that `)`
    /// is reported and owned by the header.
    fn parse_header_closing(&mut self, s: &mut StatementParts, owner: &'static str) {
        loop {
            let token = self.cursor.current();
            if is_operator(token, OperatorTokenType::ClosingParenthesis) {
                self.own_statement_token(s);
                return;
            }
            self.report_missing_operator(OperatorTokenType::ClosingParenthesis, owner);
            if is_operator(token, OperatorTokenType::Semicolon)
                && is_operator(
                    self.cursor.following(),
                    OperatorTokenType::ClosingParenthesis,
                )
            {
                self.own_statement_token(s);
                continue;
            }
            return;
        }
    }

    /// C99 §6.8.5p1: `do statement while ( expression ) ;`, after `do`.
    fn parse_do(&mut self, s: &mut StatementParts) -> Option<Built<'tu>> {
        let body = self.parse_statement(Some(ScopeKind::ImplicitIteration), false, s.leave_else)?;
        self.merge_child_statement(s, body);
        let token = self.cursor.current();
        if token
            .is_some_and(|token| matches!(token.kind, TokenType::Keyword(KeywordTokenType::While)))
        {
            self.own_statement_token(s);
        } else {
            self.report(
                ParserErrorType::ExpectedWhileAfterDoBody(token.map(|token| token.kind)),
                token,
            );
            // Without `while (` the following tokens belong to the next
            // statement, not to a condition that was never written, so
            // finish here without consuming them. When only the keyword is
            // missing, the condition follows.
            if !is_operator(token, OperatorTokenType::OpeningParenthesis) {
                let condition_expression = ExpressionSlot::Missing(self.missing_syntax_source());
                return Some(Built::Kind(StatementType::DoWhile {
                    condition_expression,
                    body_statement: body,
                }));
            }
        }
        if is_operator(self.cursor.current(), OperatorTokenType::OpeningParenthesis) {
            self.own_statement_token(s);
        } else {
            self.report_missing_operator(
                OperatorTokenType::OpeningParenthesis,
                "do-while statement",
            );
        }
        let token = self.cursor.current();
        let expression = if at_expression_boundary(token, ExpressionTerminator::ClosingParenthesis)
        {
            self.report(
                ParserErrorType::ExpectedStatementExpression(
                    "do-while statement",
                    token.map(|token| token.kind),
                ),
                token,
            );
            ExpressionSlot::Missing(self.missing_syntax_source())
        } else {
            self.parse_statement_expression_slot(s, ExpressionBoundary::ClosingParenthesis)?
        };
        self.parse_header_closing(s, "do-while statement");
        self.close_statement_part(s, OperatorTokenType::Semicolon, "do-while statement");
        Some(Built::Kind(StatementType::DoWhile {
            condition_expression: expression,
            body_statement:       body,
        }))
    }

    /// C99 §6.8.5p1: `for ( clause ; expression(opt) ; expression(opt) )
    /// statement`, after `for`. The declaration clause of §6.8.5.3p1 declares
    /// names in scope for the rest of the header and the body.
    fn parse_for(&mut self, s: &mut StatementParts) -> Option<Built<'tu>> {
        if is_operator(self.cursor.current(), OperatorTokenType::OpeningParenthesis) {
            self.own_statement_token(s);
        } else {
            self.report_missing_operator(OperatorTokenType::OpeningParenthesis, "for statement");
        }
        let token = self.cursor.current();
        let initializer = if is_operator(token, OperatorTokenType::Semicolon) {
            self.own_statement_token(s);
            None
        } else if token.is_none() || is_operator(token, OperatorTokenType::ClosingParenthesis) {
            self.report_missing_operator(OperatorTokenType::Semicolon, "for initializer");
            None
        } else if token.is_some_and(|token| self.declaration_starter(token))
            && self.extension_precedes_declaration()
        {
            // A GNU `__extension__` before an expression leaves it an
            // expression.
            self.extension(
                Feature::ForDeclarations,
                "for declaration",
                token.expect("declaration starts here"),
            );
            let declaration = self.parse_block_declaration(DeclarationContext::ForInitializer)?;
            self.context
                .merge_into(&mut s.source, declaration.source_vectors);
            // A for-initializer declaration ends at the header's `)` without
            // reporting when its own `;` is missing; that `;` belongs to the
            // initializer, not to the condition.
            if is_operator(self.cursor.current(), OperatorTokenType::ClosingParenthesis)
                && !is_operator(self.cursor.previous, OperatorTokenType::Semicolon)
            {
                self.report_missing_operator(OperatorTokenType::Semicolon, "for initializer");
            }
            Some(ForInitializer::Declaration(declaration))
        } else {
            let slot = self.parse_statement_expression_slot(
                s,
                ExpressionBoundary::Statement(ExpressionTerminator::ForSemicolon),
            )?;
            self.close_statement_part(s, OperatorTokenType::Semicolon, "for initializer");
            Some(ForInitializer::Expression(slot))
        };
        let token = self.cursor.current();
        let (condition, mut iteration, header_open) =
            if is_operator(token, OperatorTokenType::OpeningCurlyBrace) {
                self.report_missing_operator(OperatorTokenType::Semicolon, "for condition");
                self.report_missing_operator(
                    OperatorTokenType::ClosingParenthesis,
                    "for statement",
                );
                (None, None, false)
            } else if is_operator(token, OperatorTokenType::Semicolon) {
                self.own_statement_token(s);
                (None, None, true)
            } else if token.is_none() || is_operator(token, OperatorTokenType::ClosingParenthesis) {
                self.report_missing_operator(OperatorTokenType::Semicolon, "for condition");
                (None, None, true)
            } else {
                let condition = self.parse_statement_expression_slot(
                    s,
                    ExpressionBoundary::Statement(ExpressionTerminator::ForSemicolon),
                )?;
                self.close_statement_part(s, OperatorTokenType::Semicolon, "for condition");
                (Some(condition), None, true)
            };
        if header_open {
            let mut closed = false;
            loop {
                let token = self.cursor.current();
                if is_operator(token, OperatorTokenType::ClosingParenthesis) {
                    self.own_statement_token(s);
                    closed = true;
                    break;
                }
                if is_operator(token, OperatorTokenType::Semicolon)
                    && is_operator(
                        self.cursor.following(),
                        OperatorTokenType::ClosingParenthesis,
                    )
                {
                    self.report_missing_operator(
                        OperatorTokenType::ClosingParenthesis,
                        "for statement",
                    );
                    self.own_statement_token(s);
                    continue;
                }
                if token.is_some() {
                    iteration = Some(self.parse_statement_expression_slot(
                        s,
                        ExpressionBoundary::ClosingParenthesis,
                    )?);
                }
                break;
            }
            if !closed {
                loop {
                    let token = self.cursor.current();
                    if is_operator(token, OperatorTokenType::ClosingParenthesis) {
                        self.own_statement_token(s);
                        break;
                    }
                    self.report_missing_operator(
                        OperatorTokenType::ClosingParenthesis,
                        "for statement",
                    );
                    if is_operator(token, OperatorTokenType::Semicolon)
                        && is_operator(
                            self.cursor.following(),
                            OperatorTokenType::ClosingParenthesis,
                        )
                    {
                        self.own_statement_token(s);
                        continue;
                    }
                    // An extra `;` clause ends the iteration expression, and
                    // balanced recovery stops before every `;`, so the rest of
                    // the header is skipped by count once lookahead has found
                    // its `)`.
                    if is_operator(token, OperatorTokenType::Semicolon)
                        && let Some(distance) = for_header_closer_distance(self)
                    {
                        for _ in 0..=distance {
                            if self.cursor.current().is_none() {
                                break;
                            }
                            self.own_statement_token(s);
                        }
                    }
                    break;
                }
            }
        }
        let body = self.parse_statement(Some(ScopeKind::ImplicitIteration), false, s.leave_else)?;
        self.merge_child_statement(s, body);
        let clauses = self.alloc_syntax_part(ForStatement {
            initializer,
            condition_expression: condition,
            iteration_expression: iteration,
            body_statement: body,
        });
        Some(Built::Kind(StatementType::For(clauses)))
    }

    /// C99: `compound-statement` and `block-item-list` §6.8.2 paragraph 1,
    /// p. 132; PDF p. 144. Declaration recognition stays typedef-sensitive.
    /// A function body predeclares `__func__` as if declared just after its
    /// `{` (§6.4.2.2 paragraph 1, p. 52; PDF p. 64).
    #[inline(never)]
    pub(super) fn parse_compound(&mut self, function_body: bool) -> Option<&'tu Statement<'tu>> {
        let active = self.enter_production(ProductionKind::CompoundStatement)?;
        let depth = self.scopes.depth();
        self.scopes.enter_scope(ScopeKind::Block);
        let mut sources = ArenaVec::new_in(self.arena);
        self.pools.source_vectors.lend(&mut sources);
        let mut items = ArenaVec::new_in(self.arena);
        self.pools.block_items.lend(&mut items);
        let result = self.parse_compound_body(function_body, &mut sources, &mut items);
        self.pools.block_items.reclaim(&mut items);
        self.pools.source_vectors.reclaim(&mut sources);
        if !self.resource_limit_reported {
            self.scopes.restore_depth(depth);
        }
        self.leave_production(active);
        result
    }

    fn parse_compound_body(
        &mut self,
        function_body: bool,
        sources: &mut ArenaVec<'_, SourceVectors>,
        items: &mut ArenaVec<'_, BlockItem<'tu>>,
    ) -> Option<&'tu Statement<'tu>> {
        let errors = self.hard_error_count;
        let token = self.cursor.current();
        if let Some(token) =
            token.filter(|token| is_operator(Some(*token), OperatorTokenType::OpeningCurlyBrace))
        {
            sources.push(token.source_vectors);
            if function_body {
                let name = *self
                    .func_name
                    .get_or_insert_with(|| self.context.string_cache.intern("__func__"));
                self.scopes.publish(name, NameClass::Ordinary);
            }
            self.consume_token();
        } else {
            self.report(
                ParserErrorType::ExpectedOpeningCurlyBraceInCompoundStatement(
                    token.map(|token| token.kind),
                ),
                token,
            );
        }
        let mut has_statement = false;
        loop {
            let token = self.cursor.current();
            let Some(token) = token else {
                self.report(
                    ParserErrorType::ExpectedClosingCurlyBraceInCompoundStatement(None),
                    None,
                );
                break;
            };
            if is_operator(Some(token), OperatorTokenType::ClosingCurlyBrace) {
                sources.push(token.source_vectors);
                self.consume_token();
                break;
            }
            // C99 §6.8.1p1: `identifier :` begins a labeled statement even
            // when the identifier names a typedef; labels have their own name
            // space (§6.2.3p1).
            let is_label = matches!(token.kind, TokenType::Identifier)
                && is_operator(self.cursor.following(), OperatorTokenType::Colon);
            if !is_label
                && self.declaration_starter(token)
                && self.extension_precedes_declaration()
                && (!self.attribute_starter(Some(token)) || self.attributes_precede_declaration())
            {
                if has_statement {
                    self.extension(
                        Feature::MixedDeclarations,
                        "mixed declarations and code",
                        token,
                    );
                }
                let declaration = self.parse_block_declaration(DeclarationContext::Block)?;
                if declaration.is_definition_head() {
                    let mut extension = declaration.declaration_specifiers.extensions;
                    let mut suppressed = false;
                    while let Some(item) = extension {
                        suppressed |= item.kind == SpecifierExtensionKind::ExtensionMarker;
                        extension = item.next;
                    }
                    if self.context.configuration.nested_function_rules()
                        == crate::configuration::RuleSet::Clang
                    {
                        let token = self.cursor.current();
                        self.report(ParserErrorType::NestedFunctionDefinition, token);
                    } else if !suppressed && let Some(token) = self.cursor.current() {
                        self.extension(
                            Feature::NestedFunctions,
                            "nested function definition",
                            token,
                        );
                    }
                    let definition =
                        self.parse_function_definition(declaration, self.hard_error_count)?;
                    sources.push(definition.source_vectors);
                    items.push(BlockItem::FunctionDefinition(definition));
                } else {
                    sources.push(declaration.source_vectors);
                    items.push(BlockItem::Declaration(declaration));
                }
            } else {
                let child = self.parse_statement(None, true, false)?;
                sources.push(child.source_vectors);
                items.push(BlockItem::Statement(child));
                // A GNU `__label__` declaration opens the block before its
                // declarations; it is not code that they follow.
                has_statement |= !matches!(child.kind, StatementType::LocalLabels(_));
            }
        }
        let stored_items = self.alloc_syntax_list(items);
        let source_vectors = self.context.merge_vector_list(sources);
        Some(self.alloc_syntax(Statement {
            kind: StatementType::Compound {
                items: stored_items,
            },
            source_vectors,
            recovered: self.hard_error_count > errors,
        }))
    }

    fn parse_block_declaration(
        &mut self,
        context: DeclarationContext,
    ) -> Option<&'tu crate::translation_phases::parsing::declaration_syntax::Declaration<'tu>> {
        self.parse_declaration(context, self.hard_error_count)
    }

    fn parse_statement_expression_slot(
        &mut self,
        s: &mut StatementParts,
        boundary: ExpressionBoundary,
    ) -> Option<ExpressionSlot<'tu>> {
        let expression = self
            .parse_expression(ExpressionMode::Expression, boundary, boundary)?
            .expression;
        self.merge_statement_slot(s, expression.source_vectors);
        Some(ExpressionSlot::Parsed(expression))
    }

    /// Consumes the current token into the statement's source.
    fn own_statement_token(&mut self, s: &mut StatementParts) {
        let token = self.cursor.current().expect("owned statement token exists");
        self.merge_source(&mut s.source, token);
        self.consume_token();
    }

    /// Ends a statement part at its `;` or `:`, or reports it missing and
    /// leaves the token to be read again.
    fn close_statement_part(
        &mut self,
        s: &mut StatementParts,
        operator: OperatorTokenType,
        position: &'static str,
    ) {
        if is_operator(self.cursor.current(), operator) {
            self.own_statement_token(s);
        } else {
            self.report_missing_operator(operator, position);
        }
    }

    fn report_missing_operator(&mut self, operator: OperatorTokenType, position: &'static str) {
        let token = self.cursor.current();
        let found = token.map(|token| token.kind);
        let error = match operator {
            | OperatorTokenType::Semicolon =>
                ParserErrorType::ExpectedSemicolonInStatement(position, found),
            | OperatorTokenType::Colon => ParserErrorType::ExpectedColonInLabel(position, found),
            | OperatorTokenType::OpeningParenthesis =>
                ParserErrorType::ExpectedOpeningParenthesisInStatement(position, found),
            | _ => ParserErrorType::ExpectedClosingParenthesisInStatement(position, found),
        };
        self.report(error, token);
    }

    fn merge_child_statement(&mut self, s: &mut StatementParts, statement: &'tu Statement<'tu>) {
        self.context
            .merge_into(&mut s.source, statement.source_vectors);
    }

    fn merge_statement_slot(&mut self, s: &mut StatementParts, source: SourceVectors) {
        if source.length() > 0 {
            self.context.merge_into(&mut s.source, source);
        }
    }
}

/// One statement's accumulated provenance and ownership flags.
struct StatementParts {
    source:     Option<SourceVectors>,
    errors:     usize,
    /// C23 permits standalone labels among compound block items, not as a
    /// selection/iteration substatement whose required statement is absent.
    block_item: bool,
    leave_else: bool,
}

/// A statement alternative's result: a kind that this statement's node
/// wraps, or a child statement (a block or an MSVC statement) that stands
/// for it.
enum Built<'tu> {
    Kind(StatementType<'tu>),
    Existing(&'tu Statement<'tu>),
}

/// Statements whose header is `( expression )` followed by one substatement.
///
/// C99: `if` and `switch` are §6.8.4, p. 133; PDF p. 145; `while` is
/// §6.8.5, p. 135; PDF p. 147.
#[derive(Clone, Copy, PartialEq, Eq)]
enum HeaderKind {
    If,
    Switch,
    While,
}

impl HeaderKind {
    /// The implicit block each substatement forms.
    ///
    /// C99: §6.8.4 paragraph 3, p. 133; PDF p. 145; §6.8.5 paragraph 5,
    /// p. 135; PDF p. 147.
    fn scope_kind(self) -> ScopeKind {
        match self {
            | Self::If | Self::Switch => ScopeKind::ImplicitSelection,
            | Self::While => ScopeKind::ImplicitIteration,
        }
    }

    fn name(self) -> &'static str {
        match self {
            | Self::If => "if statement",
            | Self::Switch => "switch statement",
            | Self::While => "while statement",
        }
    }
}

/// The prefix of a `labeled-statement`: `identifier :`,
/// `case constant-expression :`, or `default :`.
///
/// C99: §6.8.1 paragraph 1, p. 131; PDF p. 143.
#[derive(Clone, Copy)]
enum LabelPrefix<'tu> {
    Identifier(Identifier),
    Case(ConstantExpressionSlot<'tu>),
    Range(ConstantExpressionSlot<'tu>, ConstantExpressionSlot<'tu>),
    Default,
}

/// Returns whether a token begins a statement production rather than an
/// expression that the expression production can consume.
///
/// C99: the keywords that open `labeled-statement`, `selection-statement`,
/// `iteration-statement`, and `jump-statement` in §6.8.1-§6.8.6,
/// pp. 131-136; PDF pp. 143-148, plus `else` from §6.8.4, p. 133;
/// PDF p. 145.
pub(in crate::translation_phases::parsing) fn is_statement_keyword(token: TokenType) -> bool {
    matches!(
        token,
        TokenType::Keyword(
            KeywordTokenType::Break
                | KeywordTokenType::Case
                | KeywordTokenType::Continue
                | KeywordTokenType::Default
                | KeywordTokenType::Do
                | KeywordTokenType::Else
                | KeywordTokenType::For
                | KeywordTokenType::Goto
                | KeywordTokenType::If
                | KeywordTokenType::Return
                | KeywordTokenType::Switch
                | KeywordTokenType::While
        )
    )
}

/// Tokens a malformed statement header may span before recovery stops
/// looking for its closing parenthesis.
const HEADER_RECOVERY_LOOKAHEAD: u16 = 256;

pub(in crate::translation_phases::parsing) fn at_expression_boundary(
    token: Option<Token>,
    terminator: ExpressionTerminator,
) -> bool {
    token.is_none_or(|token| {
        SynchronizationKind::StatementExpression(terminator).stops_before(token.kind)
    })
}

/// Counts the tokens from the current extra `;` of a `for` header up to
/// the header's `)`.
///
/// Extra `;`-separated clauses are admitted outside parentheses. The scan
/// gives up at any token where skipping on could swallow the body or a
/// following statement (a brace, a statement keyword, or a declaration
/// starter outside parentheses), at the end of input, or after
/// [`HEADER_RECOVERY_LOOKAHEAD`] tokens.
pub(in crate::translation_phases::parsing) fn for_header_closer_distance(
    parser: &mut Parser<'_, '_, '_>,
) -> Option<u16> {
    let mut depth = 0_usize;
    let mut token = parser.cursor.current();
    for distance in 0..HEADER_RECOVERY_LOOKAHEAD {
        let current = token?;
        match current.kind {
            | TokenType::Operator(OperatorTokenType::OpeningParenthesis) => depth += 1,
            | TokenType::Operator(OperatorTokenType::ClosingParenthesis) => {
                if depth == 0 {
                    return Some(distance);
                }
                depth -= 1;
            },
            | TokenType::Operator(
                OperatorTokenType::OpeningCurlyBrace | OperatorTokenType::ClosingCurlyBrace,
            ) => return None,
            | TokenType::Operator(OperatorTokenType::Semicolon) if depth > 0 => return None,
            | kind if is_statement_keyword(kind) => return None,
            | _ if depth == 0 && parser.declaration_starter(current) => return None,
            | _ => {},
        }
        token = parser.cursor.lookahead(usize::from(distance));
    }
    None
}
