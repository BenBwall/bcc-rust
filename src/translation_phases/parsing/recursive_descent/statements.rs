//! Recursive statement and block grammar for the phase-7 study.
//!
//! C99: statement alternatives §6.8, p. 131; PDF p. 143; compound block
//! items §6.8.2, p. 132; PDF p. 144; selection §6.8.4, p. 133; PDF p. 145;
//! iteration §6.8.5, p. 135; PDF p. 147; jumps §6.8.6, p. 136; PDF p. 148.
//! Declarations and extension statements use existing frames. Typedef scopes
//! and function-local label/switch bookkeeping are shared with those frames;
//! statement type and control-flow constraints belong to semantic analysis.

use super::StudyMode;
use crate::{
    configuration::Feature,
    translation_phases::{
        SourceVectors,
        parsing::{
            Parser,
            errors::ParserErrorType,
            frames::{
                compound_statement::CompoundStatementFrame,
                declaration::{
                    DeclarationContext,
                    DeclarationFrame,
                },
                expression::{
                    ExpressionBoundary,
                    ExpressionMode,
                },
                expression_operators::is_operator,
                function_definition::FunctionDefinitionFrame,
                statement::StatementFrame,
            },
            machine::{
                ParseFrame,
                ParseFrameKind,
                ParseValue,
            },
            recovery::ExpressionTerminator,
            scope::{
                NameClass,
                ScopeKind,
                SwitchScope,
            },
            syntax::{
                BlockItem,
                CaseRange,
                ConstantExpression,
                ConstantExpressionSlot,
                ExpressionSlot,
                ForInitializer,
                ForStatement,
                Identifier,
                Statement,
                StatementType,
            },
        },
        preprocessing::{
            KeywordTokenType,
            OperatorTokenType,
            TokenType,
        },
    },
    util::bump::ArenaVec,
};

impl<'tu> Parser<'_, 'tu, '_> {
    /// C99: statements §6.8 paragraph 1, p. 131; PDF p. 143. Each recursive
    /// substatement restores precisely the scope depth it observed on entry.
    #[inline(never)]
    pub(super) fn rd_statement(
        &mut self,
        scope: Option<ScopeKind>,
        block_item: bool,
        leave_else: bool,
    ) -> Option<&'tu Statement<'tu>> {
        if self.study_fallback()? {
            return self.rd_statement_fallback(scope, block_item, leave_else, true);
        }
        if self.study.mode == StudyMode::Growing {
            stacker::maybe_grow(64 * 1024, 1024 * 1024, || {
                self.rd_statement_entered(scope, block_item, leave_else)
            })
        } else {
            self.rd_statement_entered(scope, block_item, leave_else)
        }
    }

    fn rd_statement_entered(
        &mut self,
        scope: Option<ScopeKind>,
        block_item: bool,
        leave_else: bool,
    ) -> Option<&'tu Statement<'tu>> {
        let marker = 0_u8;
        let address = if self.study.stack_samples {
            std::ptr::from_ref(&marker).addr()
        } else {
            0
        };
        let active = self.study_enter(ParseFrameKind::Statement, address);
        self.study.stats.statement_entries += 1;
        let depth = self.scopes.depth();
        if let Some(scope) = scope {
            self.scopes.enter_scope(scope);
        }
        let result = self.rd_statement_body(block_item, leave_else);
        if !self.resource_limit_reported {
            self.scopes.restore_depth(depth);
        }
        self.study_leave(active);
        result
    }

    fn rd_statement_body(
        &mut self,
        block_item: bool,
        leave_else: bool,
    ) -> Option<&'tu Statement<'tu>> {
        let errors = self.hard_error_count;
        let token = self.cursor.current();
        if self.attribute_starter(token)
            || token.is_none()
            || token.is_some_and(|token| {
                matches!(token.kind, TokenType::Keyword(KeywordTokenType::Else))
            })
        {
            return self.rd_statement_fallback(None, block_item, leave_else, false);
        }
        let token = token.expect("statement token was checked");
        if is_operator(Some(token), OperatorTokenType::OpeningCurlyBrace) {
            return self.rd_compound(false);
        }
        let keyword = if let TokenType::Keyword(keyword) = token.kind {
            Some(keyword)
        } else {
            None
        };
        // Keep vendor statements and newer selection-header declarations in
        // their existing grammar owners. Their expression children still use
        // RD.
        if matches!(
            keyword,
            Some(KeywordTokenType::If | KeywordTokenType::Switch)
        ) && self
            .cursor
            .lookahead(1)
            .is_some_and(|lookahead| self.declaration_starter(lookahead))
            || matches!(
                keyword,
                Some(KeywordTokenType::Break | KeywordTokenType::Continue)
            ) && self
                .cursor
                .following()
                .is_some_and(|lookahead| matches!(lookahead.kind, TokenType::Identifier))
            || matches!(keyword, Some(KeywordTokenType::Goto))
                && !self
                    .cursor
                    .following()
                    .is_some_and(|lookahead| matches!(lookahead.kind, TokenType::Identifier))
        {
            return self.rd_statement_fallback(None, block_item, leave_else, false);
        }
        let mut source = token.source_vectors;
        let kind = match keyword {
            | Some(KeywordTokenType::If | KeywordTokenType::Switch | KeywordTokenType::While) => {
                let keyword = keyword.expect("selection or iteration keyword exists");
                let scope = if keyword == KeywordTokenType::While {
                    ScopeKind::ImplicitIteration
                } else {
                    ScopeKind::ImplicitSelection
                };
                self.scopes.enter_scope(scope);
                let switch = keyword == KeywordTokenType::Switch;
                if switch {
                    self.switch_scopes.push(SwitchScope::default());
                }
                self.rd_consume();
                let owner = match keyword {
                    | KeywordTokenType::If => "if statement",
                    | KeywordTokenType::Switch => "switch statement",
                    | _ => "while statement",
                };
                self.rd_append_expected(&mut source, OperatorTokenType::OpeningParenthesis, owner);
                let condition = self.rd_header_slot(&mut source, owner)?;
                self.rd_append_expected(&mut source, OperatorTokenType::ClosingParenthesis, owner);
                let body = self.rd_statement(
                    Some(scope),
                    false,
                    keyword == KeywordTokenType::If || leave_else,
                )?;
                source = self.context.merge_vectors(source, body.source_vectors);
                if switch {
                    let _scope = self.switch_scopes.pop();
                    StatementType::Switch {
                        condition_expression: condition,
                        body_statement:       body,
                    }
                } else if keyword == KeywordTokenType::While {
                    StatementType::While {
                        condition_expression: condition,
                        body_statement:       body,
                    }
                } else {
                    let else_statement = if let Some(otherwise) =
                        self.cursor.current().filter(|token| {
                            matches!(token.kind, TokenType::Keyword(KeywordTokenType::Else))
                        }) {
                        source = self.context.merge_vectors(source, otherwise.source_vectors);
                        self.rd_consume();
                        let child = self.rd_statement(
                            Some(ScopeKind::ImplicitSelection),
                            false,
                            leave_else,
                        )?;
                        source = self.context.merge_vectors(source, child.source_vectors);
                        Some(child)
                    } else {
                        None
                    };
                    StatementType::If {
                        condition_expression: condition,
                        then_statement: body,
                        else_statement,
                    }
                }
            },
            | Some(KeywordTokenType::Do) => {
                self.scopes.enter_scope(ScopeKind::ImplicitIteration);
                self.rd_consume();
                let body =
                    self.rd_statement(Some(ScopeKind::ImplicitIteration), false, leave_else)?;
                source = self.context.merge_vectors(source, body.source_vectors);
                let following = self.cursor.current();
                if let Some(while_token) = following.filter(|token| {
                    matches!(token.kind, TokenType::Keyword(KeywordTokenType::While))
                }) {
                    source = self
                        .context
                        .merge_vectors(source, while_token.source_vectors);
                    self.rd_consume();
                } else {
                    self.report(
                        ParserErrorType::ExpectedWhileAfterDoBody(following.map(|x| x.kind)),
                        following,
                    );
                    if !is_operator(following, OperatorTokenType::OpeningParenthesis) {
                        let missing = self.missing_syntax_source();
                        return Some(self.alloc_syntax(Statement {
                            kind:           StatementType::DoWhile {
                                condition_expression: ExpressionSlot::Missing(missing),
                                body_statement:       body,
                            },
                            source_vectors: source,
                            recovered:      true,
                        }));
                    }
                }
                self.rd_append_expected(
                    &mut source,
                    OperatorTokenType::OpeningParenthesis,
                    "do-while statement",
                );
                let condition = self.rd_header_slot(&mut source, "do-while statement")?;
                self.rd_append_expected(
                    &mut source,
                    OperatorTokenType::ClosingParenthesis,
                    "do-while statement",
                );
                self.rd_append_expected(
                    &mut source,
                    OperatorTokenType::Semicolon,
                    "do-while statement",
                );
                StatementType::DoWhile {
                    condition_expression: condition,
                    body_statement:       body,
                }
            },
            | Some(KeywordTokenType::For) => {
                self.scopes.enter_scope(ScopeKind::ImplicitIteration);
                self.rd_consume();
                self.rd_append_expected(
                    &mut source,
                    OperatorTokenType::OpeningParenthesis,
                    "for statement",
                );
                let initializer =
                    if is_operator(self.cursor.current(), OperatorTokenType::Semicolon) {
                        self.rd_append_expected(
                            &mut source,
                            OperatorTokenType::Semicolon,
                            "for initializer",
                        );
                        None
                    } else if self
                        .cursor
                        .current()
                        .is_some_and(|current| self.declaration_starter(current))
                        && self.extension_precedes_declaration()
                    {
                        if let Some(token) = self.cursor.current() {
                            self.extension(Feature::ForDeclarations, "for declaration", token);
                        }
                        let frame = DeclarationFrame::new(
                            self.arena,
                            DeclarationContext::ForInitializer,
                            self.hard_error_count,
                        );
                        let Some(ParseValue::Declaration(declaration)) =
                            self.study_frame(ParseFrame::Declaration(frame), false)
                        else {
                            return None;
                        };
                        source = self
                            .context
                            .merge_vectors(source, declaration.source_vectors);
                        Some(ForInitializer::Declaration(declaration))
                    } else {
                        let slot = self.rd_slot(&mut source, ExpressionTerminator::ForSemicolon)?;
                        self.rd_append_expected(
                            &mut source,
                            OperatorTokenType::Semicolon,
                            "for initializer",
                        );
                        Some(ForInitializer::Expression(slot))
                    };
                let condition = if is_operator(self.cursor.current(), OperatorTokenType::Semicolon)
                {
                    None
                } else {
                    Some(self.rd_slot(&mut source, ExpressionTerminator::ForSemicolon)?)
                };
                self.rd_append_expected(&mut source, OperatorTokenType::Semicolon, "for condition");
                let iteration =
                    if is_operator(self.cursor.current(), OperatorTokenType::ClosingParenthesis) {
                        None
                    } else {
                        Some(self.rd_slot(&mut source, ExpressionTerminator::ClosingParenthesis)?)
                    };
                let extra_clause =
                    if is_operator(self.cursor.current(), OperatorTokenType::Semicolon) {
                        StatementFrame::for_header_closer_distance(self)
                    } else {
                        None
                    };
                self.rd_append_expected(
                    &mut source,
                    OperatorTokenType::ClosingParenthesis,
                    "for statement",
                );
                if let Some(distance) = extra_clause {
                    for _ in 0..=distance {
                        let token = self.cursor.current().expect("header closer was found");
                        source = self.context.merge_vectors(source, token.source_vectors);
                        self.rd_consume();
                    }
                }
                let body =
                    self.rd_statement(Some(ScopeKind::ImplicitIteration), false, leave_else)?;
                source = self.context.merge_vectors(source, body.source_vectors);
                StatementType::For(self.alloc_syntax_part(ForStatement {
                    initializer,
                    condition_expression: condition,
                    iteration_expression: iteration,
                    body_statement: body,
                }))
            },
            | Some(KeywordTokenType::Return) => {
                self.rd_consume();
                let expression = if is_operator(self.cursor.current(), OperatorTokenType::Semicolon)
                {
                    None
                } else {
                    Some(self.rd_slot(&mut source, ExpressionTerminator::Semicolon)?)
                };
                self.rd_append_expected(
                    &mut source,
                    OperatorTokenType::Semicolon,
                    "return statement",
                );
                StatementType::Return(expression)
            },
            | Some(KeywordTokenType::Break | KeywordTokenType::Continue) => {
                self.rd_consume();
                self.rd_append_expected(
                    &mut source,
                    OperatorTokenType::Semicolon,
                    if keyword == Some(KeywordTokenType::Break) {
                        "break statement"
                    } else {
                        "continue statement"
                    },
                );
                if keyword == Some(KeywordTokenType::Break) {
                    StatementType::Break
                } else {
                    StatementType::Continue
                }
            },
            | Some(KeywordTokenType::Goto) => {
                self.rd_consume();
                let target = self
                    .cursor
                    .current()
                    .expect("goto target was checked before consuming keyword");
                let identifier = Identifier::from_token(target);
                if let Some(labels) = self.label_scopes.innermost_mut() {
                    _ = labels.references.insert(identifier.name);
                }
                source = self.context.merge_vectors(source, target.source_vectors);
                self.rd_consume();
                self.rd_append_expected(
                    &mut source,
                    OperatorTokenType::Semicolon,
                    "goto statement",
                );
                StatementType::Goto(identifier)
            },
            | Some(KeywordTokenType::Case | KeywordTokenType::Default) => {
                let default = keyword == Some(KeywordTokenType::Default);
                if default
                    && self.switch_scopes.len() > self.switch_floor
                    && let Some(switch) = self.switch_scopes.last_mut()
                {
                    if switch.has_default {
                        self.report(ParserErrorType::DuplicateDefaultLabel, Some(token));
                    } else {
                        switch.has_default = true;
                    }
                }
                self.rd_consume();
                let lower = if default {
                    None
                } else {
                    Some(self.rd_case_value(&mut source)?)
                };
                let upper = if let Some(ellipsis) = self
                    .cursor
                    .current()
                    .filter(|token| is_operator(Some(*token), OperatorTokenType::Ellipsis))
                {
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
                    source = self.context.merge_vectors(source, ellipsis.source_vectors);
                    self.rd_consume();
                    Some(self.rd_case_value(&mut source)?)
                } else {
                    None
                };
                self.rd_append_expected(
                    &mut source,
                    OperatorTokenType::Colon,
                    if default {
                        "default label"
                    } else if upper.is_some() {
                        "case range"
                    } else {
                        "case label"
                    },
                );
                let body = self.rd_labeled_body(&mut source, block_item, leave_else)?;
                match (lower, upper) {
                    | (Some(lower), Some(upper)) =>
                        StatementType::CaseRange(self.alloc_syntax(CaseRange {
                            lower,
                            upper,
                            statement: body,
                        })),
                    | (Some(lower), None) => StatementType::Case(lower, body),
                    | _ => StatementType::Default(body),
                }
            },
            | _ if matches!(token.kind, TokenType::Identifier)
                && is_operator(self.cursor.following(), OperatorTokenType::Colon) =>
            {
                let identifier = Identifier::from_token(token);
                if let Some(labels) = self.label_scopes.innermost_mut() {
                    _ = labels.definitions.insert(identifier.name);
                }
                self.rd_consume();
                self.rd_append_expected(&mut source, OperatorTokenType::Colon, "identifier label");
                let child = self.rd_labeled_body(&mut source, block_item, leave_else)?;
                StatementType::Label(identifier, child)
            },
            | _ if is_operator(Some(token), OperatorTokenType::Semicolon) => {
                self.rd_consume();
                StatementType::Null
            },
            | _ => {
                // Vendor statements, declaration-shaped recovery, and unowned
                // closers remain with the established statement recovery owner.
                if matches!(token.kind, TokenType::Keyword(_))
                    && !matches!(
                        keyword,
                        Some(
                            KeywordTokenType::Sizeof
                                | KeywordTokenType::Real
                                | KeywordTokenType::Imag
                                | KeywordTokenType::Extension
                                | KeywordTokenType::Generic
                                | KeywordTokenType::Alignof
                                | KeywordTokenType::Countof
                                | KeywordTokenType::True
                                | KeywordTokenType::False
                                | KeywordTokenType::Nullptr
                        )
                    )
                    || matches!(
                        token.kind,
                        TokenType::Operator(
                            OperatorTokenType::ClosingCurlyBrace
                                | OperatorTokenType::ClosingParenthesis
                                | OperatorTokenType::ClosingSquareBracket
                        )
                    )
                {
                    return self.rd_statement_fallback(None, block_item, leave_else, false);
                }
                source = SourceVectors::default();
                let slot = self.rd_slot(&mut source, ExpressionTerminator::Semicolon)?;
                self.rd_append_expected(
                    &mut source,
                    OperatorTokenType::Semicolon,
                    "expression statement",
                );
                StatementType::Expression(slot)
            },
        };
        Some(self.alloc_syntax(Statement {
            kind,
            source_vectors: source,
            recovered: self.hard_error_count > errors,
        }))
    }

    /// C99: compound-statement and block-item-list §6.8.2 paragraph 1,
    /// p. 132; PDF p. 144. Declaration recognition stays typedef-sensitive.
    #[inline(never)]
    pub(super) fn rd_compound(&mut self, function_body: bool) -> Option<&'tu Statement<'tu>> {
        if self.study_fallback()? {
            let frame =
                CompoundStatementFrame::new(self.arena, self.hard_error_count, function_body);
            let Some(ParseValue::CompoundStatement(statement)) =
                self.study_frame(ParseFrame::CompoundStatement(frame), true)
            else {
                return None;
            };
            return Some(statement);
        }
        if self.study.mode == StudyMode::Growing {
            stacker::maybe_grow(64 * 1024, 1024 * 1024, || {
                self.rd_compound_entered(function_body)
            })
        } else {
            self.rd_compound_entered(function_body)
        }
    }

    fn rd_compound_entered(&mut self, function_body: bool) -> Option<&'tu Statement<'tu>> {
        let marker = 0_u8;
        let address = if self.study.stack_samples {
            std::ptr::from_ref(&marker).addr()
        } else {
            0
        };
        let active = self.study_enter(ParseFrameKind::CompoundStatement, address);
        self.study.stats.compound_entries += 1;
        let depth = self.scopes.depth();
        self.scopes.enter_scope(ScopeKind::Block);
        let result = self.rd_compound_body(function_body);
        if !self.resource_limit_reported {
            self.scopes.restore_depth(depth);
        }
        self.study_leave(active);
        result
    }

    fn rd_compound_body(&mut self, function_body: bool) -> Option<&'tu Statement<'tu>> {
        let errors = self.hard_error_count;
        let mut sources = ArenaVec::new_in(self.arena);
        self.pools.source_vectors.lend(&mut sources);
        sources.push(self.rd_expect(OperatorTokenType::OpeningCurlyBrace, "compound statement"));
        if function_body {
            let name = *self
                .func_name
                .get_or_insert_with(|| self.context.string_cache.intern("__func__"));
            self.scopes.publish(name, NameClass::Ordinary);
        }
        let mut items = ArenaVec::new_in(self.arena);
        self.pools.block_items.lend(&mut items);
        let mut has_statement = false;
        while let Some(token) = self.cursor.current() {
            if is_operator(Some(token), OperatorTokenType::ClosingCurlyBrace) {
                break;
            }
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
                let frame = DeclarationFrame::new(
                    self.arena,
                    DeclarationContext::Block,
                    self.hard_error_count,
                );
                let Some(ParseValue::Declaration(declaration)) =
                    self.study_frame(ParseFrame::Declaration(frame), false)
                else {
                    return None;
                };
                if declaration.is_definition_head() {
                    let mut extension = declaration.declaration_specifiers.extensions;
                    let mut suppressed = false;
                    while let Some(item) = extension {
                        suppressed |= item.kind
                            == super::super::modern::SpecifierExtensionKind::ExtensionMarker;
                        extension = item.next;
                    }
                    if !suppressed && let Some(token) = self.cursor.current() {
                        self.extension(
                            Feature::NestedFunctions,
                            "nested function definition",
                            token,
                        );
                    }
                    let frame = FunctionDefinitionFrame::new(
                        self.arena,
                        declaration,
                        self.hard_error_count,
                    );
                    let Some(ParseValue::FunctionDefinition(definition)) =
                        self.study_frame(ParseFrame::FunctionDefinition(frame), false)
                    else {
                        return None;
                    };
                    sources.push(definition.source_vectors);
                    items.push(BlockItem::FunctionDefinition(definition));
                } else {
                    sources.push(declaration.source_vectors);
                    items.push(BlockItem::Declaration(declaration));
                }
            } else {
                let child = self.rd_statement(None, true, false)?;
                sources.push(child.source_vectors);
                has_statement |= !matches!(child.kind, StatementType::LocalLabels(_));
                items.push(BlockItem::Statement(child));
            }
        }
        sources.push(self.rd_expect(OperatorTokenType::ClosingCurlyBrace, "compound statement"));
        let source_vectors = self.context.merge_vector_list(&sources);
        let stored_items = self.alloc_syntax_list(&mut items);
        self.pools.block_items.reclaim(&mut items);
        self.pools.source_vectors.reclaim(&mut sources);
        Some(self.alloc_syntax(Statement {
            kind: StatementType::Compound {
                items: stored_items,
            },
            source_vectors,
            recovered: self.hard_error_count > errors,
        }))
    }

    fn rd_labeled_body(
        &mut self,
        source: &mut SourceVectors,
        block_item: bool,
        leave_else: bool,
    ) -> Option<&'tu Statement<'tu>> {
        let token = self.cursor.current();
        let is_label = token.is_some_and(|token| matches!(token.kind, TokenType::Identifier))
            && is_operator(self.cursor.following(), OperatorTokenType::Colon);
        let child = if is_operator(token, OperatorTokenType::ClosingCurlyBrace) {
            if !block_item {
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
            // Existing frame gives this synthesized null child the label
            // source, but does not append it to the label a second
            // time.
            return Some(self.alloc_syntax(Statement {
                kind:           StatementType::Null,
                source_vectors: *source,
                recovered:      false,
            }));
        } else if !is_label
            && token.is_some_and(|token| self.declaration_starter(token))
            && self.extension_precedes_declaration()
            && (!self.attribute_starter(token) || self.attributes_precede_declaration())
        {
            if !block_item {
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
                token.expect("declaration starts here"),
            );
            let frame =
                DeclarationFrame::new(self.arena, DeclarationContext::Block, self.hard_error_count);
            let Some(ParseValue::Declaration(declaration)) =
                self.study_frame(ParseFrame::Declaration(frame), false)
            else {
                return None;
            };
            self.alloc_syntax(Statement {
                kind:           StatementType::Declaration(declaration),
                source_vectors: declaration.source_vectors,
                recovered:      declaration.recovered,
            })
        } else {
            self.rd_statement(None, block_item, leave_else)?
        };
        *source = self.context.merge_vectors(*source, child.source_vectors);
        Some(child)
    }

    fn rd_header_slot(
        &mut self,
        source: &mut SourceVectors,
        owner: &'static str,
    ) -> Option<ExpressionSlot<'tu>> {
        let token = self.cursor.current();
        if StatementFrame::at_expression_boundary(token, ExpressionTerminator::ClosingParenthesis) {
            self.report(
                ParserErrorType::ExpectedStatementExpression(owner, token.map(|token| token.kind)),
                token,
            );
            return Some(ExpressionSlot::Missing(self.missing_syntax_source()));
        }
        let expression = self.rd_expression(
            ExpressionMode::Expression,
            ExpressionBoundary::ClosingParenthesis,
        )?;
        *source = self
            .context
            .merge_vectors(*source, expression.source_vectors);
        Some(ExpressionSlot::Parsed(expression))
    }

    fn rd_slot(
        &mut self,
        source: &mut SourceVectors,
        terminator: ExpressionTerminator,
    ) -> Option<ExpressionSlot<'tu>> {
        let expression = self.rd_expression(
            ExpressionMode::Expression,
            ExpressionBoundary::Statement(terminator),
        )?;
        *source = self
            .context
            .merge_vectors(*source, expression.source_vectors);
        Some(ExpressionSlot::Parsed(expression))
    }

    fn rd_case_value(&mut self, source: &mut SourceVectors) -> Option<ConstantExpressionSlot<'tu>> {
        let expression = self.rd_expression(
            ExpressionMode::ConstantExpression,
            ExpressionBoundary::Statement(ExpressionTerminator::Colon),
        )?;
        *source = self
            .context
            .merge_vectors(*source, expression.source_vectors);
        Some(ConstantExpressionSlot::Parsed(ConstantExpression(
            expression,
        )))
    }

    fn rd_append_expected(
        &mut self,
        source: &mut SourceVectors,
        operator: OperatorTokenType,
        owner: &'static str,
    ) {
        let part = self.rd_expect(operator, owner);
        *source = self.context.merge_vectors(*source, part);
    }

    fn rd_statement_fallback(
        &mut self,
        scope: Option<ScopeKind>,
        block_item: bool,
        leave_else: bool,
        suspend: bool,
    ) -> Option<&'tu Statement<'tu>> {
        let frame = StatementFrame::with_else_ownership(self.hard_error_count, scope, leave_else)
            .with_block_item(block_item);
        let Some(ParseValue::Statement(statement)) =
            self.study_frame(ParseFrame::Statement(frame), suspend)
        else {
            return None;
        };
        Some(statement)
    }
}
