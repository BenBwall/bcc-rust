//! Function-definition frame.
//!
//! Translation phase 7 syntax analysis (§5.1.1.2, p. 10; PDF p. 22) of
//! `function-definition` and the old-style `declaration-list` (C99: §6.9.1
//! paragraph 1, p. 141; PDF p. 153; §A.2.4, p. 416; PDF p. 428), after the
//! external-declaration frame has parsed the head.
//!
//! The frame opens the parameters' block scope, which lasts until the end of
//! the body (§6.2.1 paragraph 4, p. 29; PDF p. 41; §6.9.1 paragraph 9,
//! p. 142; PDF p. 154), and a label name space with function scope (§6.2.1
//! paragraph 3, p. 29; PDF p. 41). Diagnosed here: a declaration list after
//! a parameter type list (§6.9.1 paragraph 5, p. 141; PDF p. 153) and a
//! missing body. Left to semantic analysis: that the declarator has function
//! type (paragraph 2), the return type (paragraph 3), the storage class
//! (paragraph 4), named parameters (paragraph 5), and the declaration-list
//! constraints of paragraph 6, all p. 141; PDF p. 153; and the parameter
//! adjustments and types of paragraph 7, p. 142; PDF p. 154. Old-style
//! definitions are obsolescent (§6.11.7, p. 163; PDF p. 175) but accepted.

use std::fmt::Debug;

use super::{
    Parser,
    compound_statement::CompoundStatementFrame,
    declaration::{
        DeclarationContext,
        DeclarationFrame,
    },
    declaration_syntax::{
        Declaration,
        Declarator,
        DirectDeclarator,
    },
    errors::ParserErrorType,
    expression_operators::is_operator,
    machine::{
        ParseAction,
        ParseFrame,
        ParseValue,
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
    },
};
use crate::{
    translation_phases::{
        SourceVectors,
        TranslationError,
        preprocessing::{
            OperatorTokenType,
            Token,
        },
    },
    util::{
        arena_list::ArenaList,
        bump::{
            ArenaVec,
            Bump,
        },
    },
};

/// Parses the optional declaration list and the body that follow a
/// function-definition head.
///
/// C99: §6.9.1 paragraph 1, p. 141; PDF p. 153.
#[derive(Debug)]
pub(super) struct FunctionDefinitionFrame<'tu, 'p> {
    phase: FunctionDefinitionPhase,
    head: &'tu Declaration<'tu>,
    pub(super) declaration_list: ArenaVec<'p, &'tu Declaration<'tu>>,
    body: Option<&'tu Statement<'tu>>,
    pub(super) source_vectors: Option<SourceVectors>,
    starting_error_count: usize,
    entry_scope_depth: Option<usize>,
    diagnosed_prototype_declaration_list: bool,
    /// Whether a declaration-list diagnostic already explained a probable
    /// missing `;` after a non-function head.
    suggested_missing_semicolon: bool,
}

/// State transitions for [`FunctionDefinitionFrame`].
///
/// C99: §6.9.1 paragraph 1, p. 141; PDF p. 153.
#[derive(Debug, Clone, Copy)]
pub(super) enum FunctionDefinitionPhase {
    Start,
    DeclarationOrBody,
    AwaitDeclaration,
    AwaitBody,
    Finish,
}

#[expect(
    clippy::missing_assert_message,
    reason = "Frame phases assert the typed driver protocol, whose mismatch already identifies \
              the invariant."
)]
impl<'tu, 'p> FunctionDefinitionFrame<'tu, 'p> {
    pub(super) fn new(
        arena: &'p Bump,
        head: &'tu Declaration<'tu>,
        starting_error_count: usize,
    ) -> Self {
        Self {
            phase: FunctionDefinitionPhase::Start,
            head,
            declaration_list: ArenaVec::new_in(arena),
            body: None,
            source_vectors: None,
            starting_error_count,
            entry_scope_depth: None,
            diagnosed_prototype_declaration_list: false,
            suggested_missing_semicolon: false,
        }
    }

    pub(super) fn step(
        &mut self,
        parser: &mut Parser<'_, 'tu, 'p>,
        token: Option<Token>,
        returned: Option<ParseValue<'tu>>,
    ) -> ParseAction<'tu, 'p> {
        match self.phase {
            | FunctionDefinitionPhase::Start => {
                debug_assert!(returned.is_none());
                let declaration = *self.head;
                let declarator = self
                    .head
                    .head_declarator()
                    .expect("a function definition has one uninitialized declarator");
                self.source_vectors = Some(declaration.source_vectors);
                self.entry_scope_depth = Some(parser.scopes.depth());
                parser.scopes.enter_scope(ScopeKind::Function);
                parser.label_scopes.enter();

                if let Some(suffix) = declarator.function_suffix() {
                    match suffix {
                        | DirectDeclarator::Function { parameter_list, .. } => {
                            for parameter in parameter_list {
                                let void_singleton = parameter_list.len() == 1
                                    && parameter.declarator.is_none()
                                    && parameter.declaration_specifiers.type_specifiers
                                        == super::declaration_syntax::TypeSpecifiers::Void;
                                if !void_singleton
                                    && parameter
                                        .declarator
                                        .and_then(Declarator::identifier)
                                        .is_none()
                                {
                                    parser.context.report_extension(
                                        crate::configuration::Feature::C23Keywords,
                                        "unnamed parameter in function definition",
                                        parameter.source_vectors,
                                    );
                                }
                            }
                            if parameter_list.is_empty() {
                                self.phase = FunctionDefinitionPhase::DeclarationOrBody;
                                return ParseAction::Reprocess;
                            }
                            // C99 §6.2.1p4: every identifier declared in the
                            // parameter declarations, including enumerators
                            // declared inside array bounds or nested
                            // specifiers, has block scope ending with the
                            // body. The parameter list retained those
                            // bindings when it saw names its declarators alone
                            // cannot rebuild.
                            if parser
                                .scopes
                                .publish_retained_bindings(list_key(&parameter_list))
                            {
                                self.phase = FunctionDefinitionPhase::DeclarationOrBody;
                                return ParseAction::Reprocess;
                            }
                            for parameter in parameter_list {
                                let scopes = &mut parser.scopes;
                                parameter
                                    .declaration_specifiers
                                    .type_specifiers
                                    .collect_bindings(&mut parser.binding_scan, |name| {
                                        scopes.publish(name, NameClass::Ordinary);
                                    });
                                if let Some(name) = parameter
                                    .declarator
                                    .and_then(Declarator::identifier)
                                    .map(|identifier| identifier.name)
                                {
                                    parser.scopes.publish(name, NameClass::Ordinary);
                                }
                            }
                        },
                        // C99 §6.9.1p6: the identifier list names the
                        // parameters the declaration list then declares.
                        | DirectDeclarator::KAndRStyleFunction { parameters } => {
                            if parameters.is_empty() {
                                self.phase = FunctionDefinitionPhase::DeclarationOrBody;
                                return ParseAction::Reprocess;
                            }
                            for identifier in parameters {
                                parser.scopes.publish(identifier.name, NameClass::Ordinary);
                            }
                        },
                        | _ => unreachable!("function suffix helper returns only function forms"),
                    }
                }
                self.phase = FunctionDefinitionPhase::DeclarationOrBody;
                ParseAction::Reprocess
            },
            | FunctionDefinitionPhase::DeclarationOrBody => {
                debug_assert!(returned.is_none());
                if is_operator(token, OperatorTokenType::OpeningCurlyBrace) {
                    self.phase = FunctionDefinitionPhase::AwaitBody;
                    ParseAction::Push(ParseFrame::CompoundStatement(CompoundStatementFrame::new(
                        parser.arena,
                        parser.hard_error_count,
                        true,
                    )))
                } else if token.is_some_and(|token| parser.declaration_starter(token)) {
                    let has_parameter_type_list = self
                        .head
                        .head_declarator()
                        .and_then(Declarator::function_suffix)
                        .is_some_and(|suffix| matches!(suffix, DirectDeclarator::Function { .. }));
                    // C99 §6.9.1p5: no declaration list follows a parameter
                    // type list.
                    if has_parameter_type_list && !self.diagnosed_prototype_declaration_list {
                        self.diagnosed_prototype_declaration_list = true;
                        parser.report(
                            ParserErrorType::DeclarationListAfterParameterTypeList,
                            token,
                        );
                    }
                    self.phase = FunctionDefinitionPhase::AwaitDeclaration;
                    ParseAction::Push(ParseFrame::Declaration(DeclarationFrame::new(
                        parser.arena,
                        DeclarationContext::OldStyleParameter,
                        parser.hard_error_count,
                    )))
                } else {
                    parser.report(
                        ParserErrorType::ExpectedFunctionBody(token.map(|token| token.kind)),
                        token,
                    );
                    // A head that is not a function declarator only became a
                    // definition because a declaration followed it; the usual
                    // cause is a missing `;` after it.
                    let head_is_function = self
                        .head
                        .head_declarator()
                        .and_then(Declarator::function_suffix)
                        .is_some();
                    if !self.suggested_missing_semicolon
                        && !head_is_function
                        && let Some(declarator) = self.head.head_declarator()
                    {
                        self.suggested_missing_semicolon = true;
                        parser.suggest_semicolon_after(declarator.source_vectors);
                    }
                    // A declaration-list without a body after a head that is
                    // not a function declarator, or that already needed
                    // recovery (an unknown `__declspec(x)` prefix reads as an
                    // identifier-list head), was most likely never a
                    // definition. Parsing the rest of the file as its body
                    // would swallow every later declaration, so end it here
                    // and let the token start the next external declaration.
                    let head_is_doubtful = !self.declaration_list.is_empty()
                        && (!head_is_function
                            || self.head.recovered
                            || self.head.declaration_specifiers.implicit_int);
                    if token.is_none() || head_is_doubtful {
                        let body_source = parser.missing_syntax_source();
                        let body = parser.alloc_syntax(Statement {
                            kind:           StatementType::Compound {
                                items: ArenaList::empty(),
                            },
                            source_vectors: body_source,
                            recovered:      true,
                        });
                        self.body = Some(body);
                        self.phase = FunctionDefinitionPhase::Finish;
                        ParseAction::Reprocess
                    } else {
                        self.phase = FunctionDefinitionPhase::AwaitBody;
                        ParseAction::Push(ParseFrame::CompoundStatement(
                            CompoundStatementFrame::new(
                                parser.arena,
                                parser.hard_error_count,
                                true,
                            ),
                        ))
                    }
                }
            },
            | FunctionDefinitionPhase::AwaitDeclaration => {
                let Some(ParseValue::Declaration(declaration)) = returned else {
                    panic!("old-style declaration returned an unexpected value: {returned:?}");
                };
                let source = declaration.source_vectors;
                self.source_vectors = Some(self.source_vectors.map_or(source, |existing| {
                    parser.context.merge_vectors(existing, source)
                }));
                self.declaration_list.push(declaration);
                // A head that is not a function declarator only became a
                // definition because this declaration followed it. When the
                // declaration itself then failed, the usual cause is a
                // missing `;` after the head: say so on its diagnostic,
                // unless that diagnostic already proposes its own insertion
                // point (a `;` missing after this declaration as well).
                if declaration.recovered
                    && !self.suggested_missing_semicolon
                    && matches!(
                        parser.context.pending_errors.back(),
                        Some(TranslationError::Parsing(error)) if error.insertion_point.is_none()
                    )
                    && let Some(declarator) = self.head.head_declarator()
                    && declarator.function_suffix().is_none()
                {
                    self.suggested_missing_semicolon = true;
                    parser.suggest_semicolon_after(declarator.source_vectors);
                }
                self.phase = FunctionDefinitionPhase::DeclarationOrBody;
                ParseAction::Reprocess
            },
            | FunctionDefinitionPhase::AwaitBody => {
                let Some(ParseValue::CompoundStatement(body)) = returned else {
                    panic!("function body returned an unexpected value: {returned:?}");
                };
                let source = body.source_vectors;
                self.source_vectors = Some(self.source_vectors.map_or(source, |existing| {
                    parser.context.merge_vectors(existing, source)
                }));
                self.body = Some(body);
                self.phase = FunctionDefinitionPhase::Finish;
                ParseAction::Reprocess
            },
            | FunctionDefinitionPhase::Finish => {
                debug_assert!(returned.is_none());
                let head = *self.head;
                let declarator = self
                    .head
                    .head_declarator()
                    .expect("function definition head remains available");
                let declaration_start = parser.alloc_syntax_list(&mut self.declaration_list);
                let recovered = parser.hard_error_count > self.starting_error_count;
                let index = parser.alloc_syntax(FunctionDefinition {
                    declaration_specifiers: head.declaration_specifiers,
                    declarator,
                    declaration_list: declaration_start,
                    body: self.body.expect("function definition has a body node"),
                    source_vectors: self.source_vectors.unwrap_or_default(),
                    recovered,
                });
                parser.scopes.restore_depth(
                    self.entry_scope_depth
                        .expect("function definition entered function scope"),
                );
                let closed = parser.label_scopes.exit();
                assert!(closed, "function definition owns a label namespace");
                ParseAction::Reduce(ParseValue::FunctionDefinition(index))
            },
        }
    }
}
