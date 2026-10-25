//! Recursive external-declaration, declaration, and function-definition
//! grammar for the phase-7 study.
//!
//! Translation phase 7 syntax analysis (§5.1.1.2, p. 10; PDF p. 22) of one
//! `external-declaration` of a `translation-unit` (C99: §6.9 paragraph 1,
//! p. 140; PDF p. 152; §A.2.4, p. 416; PDF p. 428), of `declaration`,
//! `init-declarator-list`, and `init-declarator` (§6.7 paragraph 1, p. 97;
//! PDF p. 109; §A.2.2, p. 411; PDF p. 423) at file scope, in blocks, in `for`
//! clauses, and in old-style parameter declaration lists, and of
//! `function-definition` with its old-style `declaration-list` (§6.9.1
//! paragraph 1, p. 141; PDF p. 153). Both external alternatives begin with
//! `declaration-specifiers` and usually a declarator, so a declaration is
//! parsed first and becomes a function definition when a body or
//! declaration list follows its sole declarator.
//!
//! Each declarator's identifier enters scope as soon as the declarator ends
//! (§6.2.1 paragraph 7, p. 30; PDF p. 42), as a typedef name under a
//! `typedef` specifier (§6.7.7 paragraph 3, p. 123; PDF p. 135). GNU
//! `__auto_type` instead defers the binding until after its initializer. A
//! function definition opens the parameters' block scope, which lasts until
//! the end of the body (§6.2.1 paragraph 4, p. 29; PDF p. 41; §6.9.1
//! paragraph 9, p. 142; PDF p. 154), and a label name space with function
//! scope (§6.2.1 paragraph 3, p. 29; PDF p. 41).
//!
//! Diagnosed here: a typedef without declarators (§6.7 paragraph 2, p. 97;
//! PDF p. 109), a declaration list after a parameter type list (§6.9.1
//! paragraph 5, p. 141; PDF p. 153), and a missing body. The remaining
//! constraints of §6.7 paragraphs 2-7, pp. 97-98; PDF pp. 109-110, §6.9
//! paragraphs 2-5, p. 140; PDF p. 152, and §6.9.1 paragraphs 2-7,
//! pp. 141-142; PDF pp. 153-154, are left to semantic analysis. Old-style
//! definitions are obsolescent (§6.11.7, p. 163; PDF p. 175) but accepted.

use crate::{
    configuration::Feature,
    translation_phases::{
        ErrorSeverity,
        SourceVectors,
        TranslationError,
        parsing::{
            ExternalDeclaration,
            Parser,
            declaration_syntax::{
                Declaration,
                DeclarationSpecifiers,
                Declarator,
                DirectDeclarator,
                InitDeclarator,
                TypeSpecifiers,
            },
            errors::{
                DeclarationContinuation,
                DeclarationPlace,
                ParserErrorType,
                RecoverySummary,
                RelatedParserDiagnostic,
            },
            extensions::modern::{
                ExtendedType,
                SpecifierExtension,
                SpecifierExtensionKind,
            },
            grammar::{
                ProductionKind,
                declaration_specifiers::SpecifierMode,
                declarators::DeclaratorMode,
                expression_operators::is_operator,
                gnu::{
                    GnuKind,
                    GnuValue,
                },
                modern::{
                    ModernKind,
                    ModernValue,
                },
                statements::is_statement_keyword,
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
            syntax::{
                FunctionDefinition,
                Statement,
                StatementType,
                StorageClass,
            },
        },
        preprocessing::{
            KeywordTokenType,
            OperatorTokenType,
            TokenType,
        },
    },
    util::{
        arena_list::ArenaList,
        bump::ArenaVec,
    },
};

impl<'tu, 'p> Parser<'_, 'tu, 'p> {
    /// Parses one external declaration: file-scope `asm`, a declaration, or
    /// a function definition, classified by the hard errors reported since
    /// `starting_error_count`. Root-level recovery context attaches to the
    /// first unrecovered error after `starting_diagnostic_count`.
    ///
    /// C99: external-declaration, §6.9 paragraph 1, p. 140; PDF p. 152;
    /// its file-scope position is paragraph 4, p. 140; PDF p. 152.
    pub(in crate::translation_phases::parsing) fn rd_external_declaration(
        &mut self,
        starting_error_count: usize,
        starting_diagnostic_count: usize,
    ) -> Option<ExternalDeclaration<'tu>> {
        let entry = self.enter_production(ProductionKind::ExternalDeclaration)?;
        let result =
            self.rd_external_declaration_body(starting_error_count, starting_diagnostic_count);
        self.leave_production(entry);
        result
    }

    fn rd_external_declaration_body(
        &mut self,
        starting_error_count: usize,
        starting_diagnostic_count: usize,
    ) -> Option<ExternalDeclaration<'tu>> {
        if self.cursor.current().is_some_and(|token| {
            matches!(token.kind, TokenType::Keyword(KeywordTokenType::Asm))
                || matches!(token.kind, TokenType::Keyword(KeywordTokenType::MsAsm))
                    && token.contents == KeywordTokenType::MsAsm.cache_id()
        }) {
            let value = self.rd_gnu(GnuKind::Asm { label: false }, self.hard_error_count)?;
            let GnuValue::Asm(asm) = value else {
                panic!("file asm child protocol");
            };
            return Some(ExternalDeclaration::Asm(asm));
        }
        let declaration =
            self.rd_declaration(DeclarationContext::External, self.hard_error_count)?;
        let token = self.cursor.current();
        // C99 §6.9.1p1: `declaration-specifiers declarator
        // declaration-list? compound-statement`.
        let is_definition = declaration.is_definition_head()
            && (is_operator(token, OperatorTokenType::OpeningCurlyBrace)
                || token.is_some_and(|token| self.declaration_starter(token)));
        if is_definition {
            let definition = self.rd_function_definition(declaration, starting_error_count)?;
            return Some(if definition.recovered {
                ExternalDeclaration::RecoveredFunctionDefinition(definition)
            } else {
                ExternalDeclaration::FunctionDefinition(definition)
            });
        }
        if self.hard_error_count <= starting_error_count {
            return Some(ExternalDeclaration::Declaration(declaration));
        }
        if declaration.is_meaningful() {
            return Some(ExternalDeclaration::RecoveredDeclaration(declaration));
        }
        let declaration_source = declaration.source_vectors;
        let source = if declaration_source.length() == 0 {
            token.map_or(declaration_source, |token| token.source_vectors)
        } else {
            declaration_source
        };
        let related = token.map(|token| {
            self.context.diagnostic_slice(&[RelatedParserDiagnostic {
                message:        "parsing resumes here",
                source_vectors: token.source_vectors,
            }])
        });
        if let Some(TranslationError::Parsing(error)) = self
            .context
            .pending_errors
            .iter_mut()
            .skip(starting_diagnostic_count)
            .find(|error| {
                matches!(error, TranslationError::Parsing(error)
                    if error.severity == ErrorSeverity::Error && error.recovery.is_none())
            })
        {
            if let Some(related) = related {
                error.related = related;
            }
            error.recovery = Some(RecoverySummary {
                owner:            ProductionKind::ExternalDeclaration,
                discarded:        None,
                discarded_tokens: 0,
                stopped_at:       token.map(|token| token.kind),
            });
        }
        Some(ExternalDeclaration::Error(source))
    }

    /// Parses a declaration shell around declaration specifiers,
    /// comma-separated declarators, optional initializers, and the
    /// terminating semicolon, or a declaration-shaped function-definition
    /// head. `context` decides how it may end and what it may hand off.
    ///
    /// C99: declaration, init-declarator-list, and init-declarator are §6.7,
    /// p. 97; PDF p. 109.
    pub(in crate::translation_phases::parsing) fn rd_declaration(
        &mut self,
        context: DeclarationContext,
        starting_error_count: usize,
    ) -> Option<&'tu Declaration<'tu>> {
        let entry = self.enter_production(ProductionKind::Declaration)?;
        let mut shell = DeclarationShell {
            context,
            starting_error_count,
            suppression_entry: self.pedantic_suppression,
            init_declarators: ArenaVec::new_in(self.arena),
            source_vectors: ArenaVec::new_in(self.arena),
            is_function_definition_head: false,
        };
        self.pools
            .init_declarators
            .lend(&mut shell.init_declarators);
        self.pools.source_vectors.lend(&mut shell.source_vectors);
        let result = self.rd_declaration_body(&mut shell);
        self.pools
            .init_declarators
            .reclaim(&mut shell.init_declarators);
        self.pools.source_vectors.reclaim(&mut shell.source_vectors);
        self.leave_production(entry);
        result
    }

    fn rd_declaration_body(
        &mut self,
        shell: &mut DeclarationShell<'tu, 'p>,
    ) -> Option<&'tu Declaration<'tu>> {
        let mut leading_extension = None;
        while let Some(token) = self
            .cursor
            .current()
            .filter(|token| matches!(token.kind, TokenType::Keyword(KeywordTokenType::Extension)))
        {
            self.pedantic_suppression += 1;
            self.merge_source(&mut leading_extension, token);
            shell.source_vectors.push(token.source_vectors);
            self.rd_consume();
        }
        if self
            .cursor
            .current()
            .is_some_and(|x| matches!(x.kind, TokenType::Keyword(KeywordTokenType::StaticAssert)))
        {
            let value = self.rd_modern(ModernKind::Assertion, None, self.hard_error_count)?;
            self.pedantic_suppression = shell.suppression_entry;
            let ModernValue::Assertion(assertion) = value else {
                unreachable!("assertion declaration protocol: {value:?}")
            };
            return Some(self.alloc_syntax(Declaration {
                assertion:                   Some(assertion),
                declaration_specifiers:      DeclarationSpecifiers::new(),
                init_declarators:            ArenaList::empty(),
                source_vectors:              assertion.source_vectors,
                recovered:                   assertion.recovered,
                is_function_definition_head: false,
            }));
        }
        // Specifiers are a child production because tag specifiers own
        // complete struct/union/enum bodies.
        let mut specifiers = self.rd_declaration_specifiers(SpecifierMode::Declaration, true)?;
        if let Some(source_vectors) = leading_extension {
            specifiers.extensions = Some(self.alloc_syntax(SpecifierExtension {
                kind: SpecifierExtensionKind::ExtensionMarker,
                next: specifiers.extensions,
                source_vectors,
            }));
            specifiers.source_vectors = self
                .context
                .merge_vectors(source_vectors, specifiers.source_vectors);
        }
        shell.source_vectors.push(specifiers.source_vectors);
        let token = self.cursor.current();
        // A bare `;` completes the grammar's optional init-declarator-list. A
        // typedef without one names no type, so retain the tree but diagnose
        // it. C99 §6.7p2 asks only for a declarator, a tag, or enumeration
        // members, so `typedef struct s { int m; };` is valid and only warned
        // about; `typedef int;` declares nothing. Other empty declarations
        // (`int;`) are left to semantic analysis.
        if let Some(semicolon) = token.filter(|token| {
            matches!(
                token.kind,
                TokenType::Operator(OperatorTokenType::Semicolon)
            )
        }) {
            if specifiers.storage_class == Some(StorageClass::Typedef) {
                let error = if specifiers.type_specifiers.may_declare_tag_or_enumerators() {
                    ParserErrorType::TypedefDeclaresNoName
                } else {
                    ParserErrorType::ExpectedDeclaratorInTypedef(Some(semicolon.kind))
                };
                self.report(error, token);
            }
            shell.source_vectors.push(semicolon.source_vectors);
            self.rd_consume();
        } else {
            self.rd_init_declarator_list(shell, specifiers)?;
        }
        self.pedantic_suppression = shell.suppression_entry;
        // Arena insertion is the reduction boundary: all child slices and
        // source ranges are complete before the declaration is stored.
        let source_vectors = self.context.merge_vector_list(&shell.source_vectors);
        let init_declarators = self.alloc_syntax_list(&mut shell.init_declarators);
        Some(self.alloc_syntax(Declaration {
            assertion: None,
            declaration_specifiers: specifiers,
            init_declarators,
            source_vectors,
            recovered: self.hard_error_count > shell.starting_error_count,
            is_function_definition_head: shell.is_function_definition_head,
        }))
    }

    /// Parses init-declarators until the declaration ends, hands off a
    /// function-definition head, or stops before a token its parent owns.
    ///
    /// C99: init-declarator-list, §6.7 paragraph 1, p. 97; PDF p. 109.
    fn rd_init_declarator_list(
        &mut self,
        shell: &mut DeclarationShell<'tu, 'p>,
        specifiers: DeclarationSpecifiers<'tu>,
    ) -> Option<()> {
        let auto_type = matches!(
            specifiers.type_specifiers,
            TypeSpecifiers::Extended(ExtendedType::AutoType)
        );
        'declarators: loop {
            let declarator = self.rd_declarator(DeclaratorMode::Named)?;
            let Some(declarator) = declarator else {
                let token = self.cursor.current();
                self.report(
                    ParserErrorType::ExpectedDeclaratorInDeclaration(token.map(|token| token.kind)),
                    token,
                );
                if self.rd_declaration_recover(shell) {
                    continue 'declarators;
                }
                return Some(());
            };
            let source_vectors = declarator.source_vectors;
            shell.source_vectors.push(source_vectors);
            shell.init_declarators.push(InitDeclarator {
                declarator,
                initializer: None,
                source_vectors,
            });
            // C scope begins immediately after the declarator, before its
            // initializer or a later comma-separated declarator. Publish now
            // so typedef shadowing affects the very next token. C99 §6.2.1p7;
            // typedef names share the ordinary name space (§6.7.7p3).
            // GNU extension: `__auto_type` keeps outer bindings visible
            // throughout its initializer (GCC's "Referring to a Type with
            // typeof" specification).
            if !auto_type && let Some(identifier) = declarator.identifier() {
                let class = if specifiers.storage_class == Some(StorageClass::Typedef) {
                    NameClass::Typedef
                } else {
                    NameClass::Ordinary
                };
                self.scopes.publish(identifier.name, class);
            }
            loop {
                match self.rd_after_declarator(shell) {
                    | AfterDeclarator::NextDeclarator => continue 'declarators,
                    | AfterDeclarator::Finish => return Some(()),
                    | AfterDeclarator::Initializer(equals_source) => {
                        let result = self.rd_initializer(
                            self.hard_error_count,
                            matches!(
                                shell.context,
                                DeclarationContext::ForInitializer
                                    | DeclarationContext::SelectionHeader
                            ),
                            false,
                        )?;
                        let initializer = result.initializer;
                        let source_vectors = initializer.source_vectors;
                        let initializer_source =
                            self.context.merge_vectors(equals_source, source_vectors);
                        shell.source_vectors.push(source_vectors);
                        if let Some(init_declarator) = shell.init_declarators.last_mut() {
                            init_declarator.initializer = Some(initializer);
                            init_declarator.source_vectors = self
                                .context
                                .merge_vectors(init_declarator.source_vectors, initializer_source);
                            // GNU `__auto_type` is an exception to C99
                            // §6.2.1p7: publish only after the initializer
                            // has finished, including when it recovered
                            // malformed syntax.
                            if auto_type
                                && let Some(identifier) = init_declarator.declarator.identifier()
                            {
                                self.scopes.publish(identifier.name, NameClass::Ordinary);
                            }
                        }
                    },
                }
            }
        }
    }

    /// Classifies the token following a completed declarator or
    /// initializer, consuming the separator it owns.
    fn rd_after_declarator(&mut self, shell: &mut DeclarationShell<'tu, 'p>) -> AfterDeclarator {
        let token = self.cursor.current();
        let has_sole_uninitialized_declarator = matches!(
            shell.init_declarators.as_slice(),
            [init] if init.initializer.is_none()
        );
        let errors_since_entry = self.hard_error_count != shell.starting_error_count;
        // An identifier-list head keeps its declaration-list even after a
        // recovered error in the head: no other declaration form can continue
        // with a declaration there, while a broken non-function declarator
        // (`int x[3 int y;`) must still resynchronize at the next declaration.
        let old_style_parameters = if has_sole_uninitialized_declarator && errors_since_entry {
            shell
                .init_declarators
                .first()
                .and_then(|init| init.declarator.function_suffix())
                .and_then(|suffix| match suffix {
                    | DirectDeclarator::KAndRStyleFunction { parameters } => Some(parameters),
                    | _ => None,
                })
        } else {
            None
        };
        // After an error in an identifier-list head, a following declaration
        // still begins the declaration list only when it declares one of the
        // listed parameters; otherwise the head most likely lacks its `;`.
        let continues_old_style_definition = errors_since_entry
            && token.is_some_and(|token| self.declaration_starter(token))
            && old_style_parameters.is_some_and(|parameters| {
                self.next_declaration_declares_one_of(parameters.as_slice())
            });
        // A GNU nested function (block context) starts at `{` after a
        // function declarator, or at a declaration-list that declares one of
        // its identifier-list parameters. A prototype head followed by a
        // declaration is a block declaration missing its `;`, not a
        // definition swallowing the rest of the block.
        let starts_function_definition = has_sole_uninitialized_declarator
            && match shell.context {
                | DeclarationContext::External =>
                    is_operator(token, OperatorTokenType::OpeningCurlyBrace)
                        || !errors_since_entry
                            && token.is_some_and(|token| self.declaration_starter(token))
                        || continues_old_style_definition,
                | DeclarationContext::Block => match shell
                    .init_declarators
                    .first()
                    .and_then(|init| init.declarator.function_suffix())
                {
                    | Some(DirectDeclarator::KAndRStyleFunction { parameters })
                        if !parameters.is_empty()
                            && !is_operator(token, OperatorTokenType::OpeningCurlyBrace) =>
                        token.is_some_and(|token| self.declaration_starter(token))
                            && self.next_declaration_declares_one_of(parameters.as_slice()),
                    | Some(_) => is_operator(token, OperatorTokenType::OpeningCurlyBrace),
                    | None => false,
                },
                | DeclarationContext::ForInitializer
                | DeclarationContext::SelectionHeader
                | DeclarationContext::OldStyleParameter => false,
            };
        // The same prefix can continue as another init-declarator, an
        // initializer, a completed declaration, or a function body. A sole
        // declarator followed by `{` or a declaration-list matches the
        // function-definition production. Whether that declarator denotes a
        // function type, and whether its form permits a declaration-list,
        // are separate C constraints (C99 §6.9.1p2 and p5-6).
        if let Some(separator) = token.filter(|token| {
            matches!(
                token.kind,
                TokenType::Operator(
                    OperatorTokenType::Comma
                        | OperatorTokenType::Semicolon
                        | OperatorTokenType::Equals
                )
            )
        }) {
            shell.source_vectors.push(separator.source_vectors);
            self.rd_consume();
            return match separator.kind {
                | TokenType::Operator(OperatorTokenType::Comma) => AfterDeclarator::NextDeclarator,
                | TokenType::Operator(OperatorTokenType::Equals) =>
                    AfterDeclarator::Initializer(separator.source_vectors),
                | _ => AfterDeclarator::Finish,
            };
        }
        if starts_function_definition {
            shell.is_function_definition_head = true;
            return AfterDeclarator::Finish;
        }
        if matches!(
            shell.context,
            DeclarationContext::ForInitializer | DeclarationContext::SelectionHeader
        ) && is_operator(token, OperatorTokenType::ClosingParenthesis)
        {
            return AfterDeclarator::Finish;
        }
        let continuation_error = ParserErrorType::ExpectedDeclarationContinuationAfterDeclarator(
            token.map(|token| token.kind),
            shell.continuation(),
        );
        if shell.context == DeclarationContext::OldStyleParameter
            && is_operator(token, OperatorTokenType::OpeningCurlyBrace)
        {
            self.report(continuation_error, token);
            return AfterDeclarator::Finish;
        }
        if let Some(close) = token.filter(|token| {
            matches!(
                token.kind,
                TokenType::Operator(OperatorTokenType::ClosingCurlyBrace)
            )
        }) {
            self.report(continuation_error, token);
            if shell.context == DeclarationContext::External {
                shell.source_vectors.push(close.source_vectors);
                self.rd_consume();
            }
            return AfterDeclarator::Finish;
        }
        if token.is_none_or(|token| {
            self.declaration_starter(token)
                || matches!(
                    shell.context,
                    DeclarationContext::Block | DeclarationContext::ForInitializer
                ) && is_statement_keyword(token.kind)
        }) || matches!(
            shell.context,
            DeclarationContext::Block | DeclarationContext::ForInitializer
        ) && (is_operator(token, OperatorTokenType::OpeningCurlyBrace)
            || token.is_some_and(|token| matches!(token.kind, TokenType::Identifier))
                && is_operator(self.cursor.following(), OperatorTokenType::Colon))
        {
            self.report(continuation_error, token);
            return AfterDeclarator::Finish;
        }
        self.report(continuation_error, token);
        if self.rd_declaration_recover(shell) {
            AfterDeclarator::NextDeclarator
        } else {
            AfterDeclarator::Finish
        }
    }

    /// Synchronizes after a missing declarator or a rejected continuation,
    /// then consumes the separator recovery stopped before, if this
    /// declaration owns it. Returns whether another declarator follows.
    fn rd_declaration_recover(&mut self, shell: &mut DeclarationShell<'tu, 'p>) -> bool {
        let set = SynchronizationSet {
            kind:   shell.recovery_kind(),
            target: ProductionKind::Declaration,
        };
        if let Some(recovered) = self.recover(set) {
            shell.source_vectors.push(recovered);
        }
        // Recovery stops before owning separators. Consume them here; leave
        // any unrelated token untouched for the parent.
        let token = self.cursor.current();
        let owned = token.filter(|token| match token.kind {
            | TokenType::Operator(OperatorTokenType::Comma | OperatorTokenType::Semicolon) => true,
            | TokenType::Operator(OperatorTokenType::ClosingCurlyBrace) =>
                shell.context == DeclarationContext::External,
            | _ => false,
        });
        if let Some(owned) = owned {
            shell.source_vectors.push(owned.source_vectors);
            self.rd_consume();
        }
        is_operator(owned, OperatorTokenType::Comma)
    }

    /// Parses the optional declaration list and the body that follow a
    /// function-definition `head`. `starting_error_count` is the enclosing
    /// external or block item's, so errors in the head mark the definition
    /// recovered.
    ///
    /// C99: §6.9.1 paragraph 1, p. 141; PDF p. 153.
    pub(in crate::translation_phases::parsing) fn rd_function_definition(
        &mut self,
        head: &'tu Declaration<'tu>,
        starting_error_count: usize,
    ) -> Option<&'tu FunctionDefinition<'tu>> {
        let entry = self.enter_production(ProductionKind::FunctionDefinition)?;
        let suppression_entry = self.pedantic_suppression;
        let mut extension = head.declaration_specifiers.extensions;
        while let Some(item) = extension {
            if item.kind == SpecifierExtensionKind::ExtensionMarker {
                self.pedantic_suppression += 1;
            }
            extension = item.next;
        }
        let switch_floor_entry = self.switch_floor;
        self.switch_floor = self.switch_scopes.len();
        let declarator = head
            .head_declarator()
            .expect("a function definition has one uninitialized declarator");
        let entry_scope_depth = self.scopes.depth();
        self.scopes.enter_scope(ScopeKind::Function);
        self.label_scopes.enter();
        self.rd_publish_parameters(declarator);

        let mut declaration_list = ArenaVec::new_in(self.arena);
        self.pools.declarations.lend(&mut declaration_list);
        let mut source_vectors = Some(head.source_vectors);
        let body = self.rd_function_body(head, &mut declaration_list, &mut source_vectors);
        let result = body.map(|body| {
            self.pedantic_suppression = suppression_entry;
            self.switch_floor = switch_floor_entry;
            let declaration_list = self.alloc_syntax_list(&mut declaration_list);
            let recovered = self.hard_error_count > starting_error_count;
            let definition = self.alloc_syntax(FunctionDefinition {
                declaration_specifiers: head.declaration_specifiers,
                declarator,
                declaration_list,
                body,
                source_vectors: source_vectors.unwrap_or_default(),
                recovered,
            });
            self.scopes.restore_depth(entry_scope_depth);
            let closed = self.label_scopes.exit();
            assert!(closed, "function definition owns a label namespace");
            definition
        });
        self.pools.declarations.reclaim(&mut declaration_list);
        self.leave_production(entry);
        result
    }

    /// Publishes the parameter names of a definition's function declarator
    /// into its function scope.
    fn rd_publish_parameters(&mut self, declarator: Declarator<'tu>) {
        match declarator.function_suffix() {
            | None => {},
            | Some(DirectDeclarator::Function { parameter_list, .. }) => {
                for parameter in parameter_list {
                    // C99 §6.9.1p5: a sole unnamed parameter of type `void`
                    // takes no identifier. A typedef name may denote `void`;
                    // whether it does is left to semantic analysis.
                    let void_singleton = parameter_list.len() == 1
                        && parameter.declarator.is_none()
                        && matches!(
                            parameter.declaration_specifiers.type_specifiers,
                            TypeSpecifiers::Void | TypeSpecifiers::TypedefName(_)
                        );
                    if !void_singleton
                        && parameter
                            .declarator
                            .and_then(Declarator::identifier)
                            .is_none()
                    {
                        self.extension_source(
                            Feature::UnnamedDefinitionParameters,
                            "unnamed parameter in function definition",
                            parameter.source_vectors,
                        );
                    }
                }
                // C99 §6.2.1p4: every identifier declared in the parameter
                // declarations, including enumerators declared inside array
                // bounds or nested specifiers, has block scope ending with
                // the body. The parameter list retained those bindings when
                // it saw names its declarators alone cannot rebuild.
                if parameter_list.is_empty()
                    || self
                        .scopes
                        .publish_retained_bindings(list_key(&parameter_list))
                {
                    return;
                }
                for parameter in parameter_list {
                    let scopes = &mut self.scopes;
                    parameter
                        .declaration_specifiers
                        .type_specifiers
                        .collect_bindings(&mut self.binding_scan, |name| {
                            scopes.publish(name, NameClass::Ordinary);
                        });
                    if let Some(name) = parameter
                        .declarator
                        .and_then(Declarator::identifier)
                        .map(|identifier| identifier.name)
                    {
                        self.scopes.publish(name, NameClass::Ordinary);
                    }
                }
            },
            // C99 §6.9.1p6: the identifier list names the parameters the
            // declaration list then declares.
            | Some(DirectDeclarator::KAndRStyleFunction { parameters }) => {
                for identifier in parameters {
                    self.scopes.publish(identifier.name, NameClass::Ordinary);
                }
            },
            | Some(_) => unreachable!("function suffix helper returns only function forms"),
        }
    }

    /// Parses the old-style declaration list and the compound body, or
    /// synthesizes a recovered empty body.
    fn rd_function_body(
        &mut self,
        head: &'tu Declaration<'tu>,
        declaration_list: &mut ArenaVec<'p, &'tu Declaration<'tu>>,
        source_vectors: &mut Option<SourceVectors>,
    ) -> Option<&'tu Statement<'tu>> {
        let head_suffix = head.head_declarator().and_then(Declarator::function_suffix);
        let mut diagnosed_prototype_declaration_list = false;
        let mut suggested_missing_semicolon = false;
        loop {
            let token = self.cursor.current();
            if is_operator(token, OperatorTokenType::OpeningCurlyBrace) {
                return self.rd_definition_compound(source_vectors);
            }
            if token.is_some_and(|token| self.declaration_starter(token)) {
                // C99 §6.9.1p5: no declaration list follows a parameter type
                // list.
                if matches!(head_suffix, Some(DirectDeclarator::Function { .. }))
                    && !diagnosed_prototype_declaration_list
                {
                    diagnosed_prototype_declaration_list = true;
                    self.report(
                        ParserErrorType::DeclarationListAfterParameterTypeList,
                        token,
                    );
                }
                let declaration = self
                    .rd_declaration(DeclarationContext::OldStyleParameter, self.hard_error_count)?;
                self.context
                    .merge_into(source_vectors, declaration.source_vectors);
                declaration_list.push(declaration);
                // A head that is not a function declarator only became a
                // definition because this declaration followed it. When the
                // declaration itself then failed, the usual cause is a
                // missing `;` after the head: say so on its diagnostic,
                // unless that diagnostic already proposes its own insertion
                // point (a `;` missing after this declaration as well).
                if declaration.recovered
                    && !suggested_missing_semicolon
                    && matches!(
                        self.context.pending_errors.back(),
                        Some(TranslationError::Parsing(error)) if error.insertion_point.is_none()
                    )
                    && let Some(declarator) = head.head_declarator()
                    && declarator.function_suffix().is_none()
                {
                    suggested_missing_semicolon = true;
                    self.suggest_semicolon_after(declarator.source_vectors);
                }
                continue;
            }
            self.report(
                ParserErrorType::ExpectedFunctionBody(token.map(|token| token.kind)),
                token,
            );
            // A head that is not a function declarator only became a
            // definition because a declaration followed it; the usual cause
            // is a missing `;` after it.
            let head_is_function = head_suffix.is_some();
            if !suggested_missing_semicolon
                && !head_is_function
                && let Some(declarator) = head.head_declarator()
            {
                self.suggest_semicolon_after(declarator.source_vectors);
            }
            // A declaration-list without a body after a head that is not a
            // function declarator, or that already needed recovery (an
            // unknown `__declspec(x)` prefix reads as an identifier-list
            // head), was most likely never a definition. Parsing the rest of
            // the file as its body would swallow every later declaration, so
            // end it here and let the token start the next external
            // declaration.
            let head_is_doubtful = !declaration_list.is_empty()
                && (!head_is_function
                    || head.recovered
                    || head.declaration_specifiers.implicit_int);
            if token.is_none() || head_is_doubtful {
                let body_source = self.missing_syntax_source();
                return Some(self.alloc_syntax(Statement {
                    kind:           StatementType::Compound {
                        items: ArenaList::empty(),
                    },
                    source_vectors: body_source,
                    recovered:      true,
                }));
            }
            return self.rd_definition_compound(source_vectors);
        }
    }

    fn rd_definition_compound(
        &mut self,
        source_vectors: &mut Option<SourceVectors>,
    ) -> Option<&'tu Statement<'tu>> {
        let body = self.rd_compound(true)?;
        self.context.merge_into(source_vectors, body.source_vectors);
        Some(body)
    }
}

/// One declaration under construction.
///
/// C99: §6.7, p. 97; PDF p. 109.
struct DeclarationShell<'tu, 'p> {
    /// Grammar context controlling function-definition handoff and `}`
    /// ownership.
    context:                     DeclarationContext,
    /// Hard-error count on entry, used to scope recovery to this declaration.
    starting_error_count:        usize,
    /// The pedantic-suppression depth on entry, restored when a leading
    /// `__extension__` goes out of scope.
    suppression_entry:           usize,
    init_declarators:            ArenaVec<'p, InitDeclarator<'tu>>,
    /// Provenance accumulated across specifiers, declarators, and separators.
    source_vectors:              ArenaVec<'p, SourceVectors>,
    is_function_definition_head: bool,
}

/// What follows a completed declarator or initializer.
enum AfterDeclarator {
    /// A `,` was consumed.
    NextDeclarator,
    /// A `=` with this provenance was consumed.
    Initializer(SourceVectors),
    /// The declaration ends.
    Finish,
}

impl DeclarationShell<'_, '_> {
    /// Tokens that may still continue this declaration after its latest
    /// declarator, for the continuation diagnostic.
    fn continuation(&self) -> DeclarationContinuation {
        let place = match self.context {
            | DeclarationContext::External => DeclarationPlace::External,
            | DeclarationContext::Block => DeclarationPlace::Block,
            | DeclarationContext::ForInitializer | DeclarationContext::SelectionHeader =>
                DeclarationPlace::ForInitializer,
            | DeclarationContext::OldStyleParameter => DeclarationPlace::OldStyleParameter,
        };
        DeclarationContinuation {
            place,
            initializer: self
                .init_declarators
                .last()
                .is_some_and(|init| init.initializer.is_none()),
            function_body: place == DeclarationPlace::External
                && matches!(
                    self.init_declarators.as_slice(),
                    [init] if init.initializer.is_none()
                ),
        }
    }

    fn recovery_kind(&self) -> SynchronizationKind {
        match self.context {
            | DeclarationContext::Block => SynchronizationKind::BlockDeclaration,
            | DeclarationContext::ForInitializer | DeclarationContext::SelectionHeader =>
                SynchronizationKind::ForInitializer,
            | DeclarationContext::External => SynchronizationKind::Declaration,
            | DeclarationContext::OldStyleParameter => SynchronizationKind::OldStyleParameter,
        }
    }
}

/// Where a declaration appears, which decides how it may end and what it
/// may hand off.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::translation_phases::parsing) enum DeclarationContext {
    /// An `external-declaration`: C99 §6.9 paragraph 1, p. 140; PDF p. 152.
    External,
    /// A `block-item`: C99 §6.8.2 paragraph 1, p. 132; PDF p. 144.
    Block,
    /// The declaration clause of `for`: C99 §6.8.5 paragraph 1, p. 135;
    /// PDF p. 147. Its storage-class constraint (paragraph 3, p. 135;
    /// PDF p. 147) is left to semantic analysis.
    ForInitializer,
    SelectionHeader,
    /// One declaration of a function definition's `declaration-list`: C99
    /// §6.9.1 paragraph 1, p. 141; PDF p. 153.
    OldStyleParameter,
}
