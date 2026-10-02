//! Function-definition frame.

use std::fmt::Debug;

use super::{
    Parser,
    compound_statement::CompoundStatementFrame,
    declaration::{
        DeclarationContext,
        DeclarationFrame,
    },
    declaration_syntax::DirectDeclarator,
    errors::ParserErrorType,
    expression_operators::is_operator,
    machine::{
        ParseAction,
        ParseFrame,
        ParseValue,
    },
    scope::{
        LabelScope,
        NameClass,
        ScopeKind,
    },
    syntax::{
        DeclarationIndex,
        FunctionDefinition,
        FunctionDefinitionIndex,
        Statement,
        StatementIndex,
        StatementType,
        SyntaxList,
    },
};
use crate::{
    translation_phases::{
        Context,
        GetPosition,
        GetSourceFileIndex,
        SourceVectors,
        preprocessing::{
            OperatorTokenType,
            Token,
        },
    },
    util::vector_slice::UsizeExt,
};

#[derive(Debug)]
pub(super) struct FunctionDefinitionFrame {
    phase: FunctionDefinitionPhase,
    head: DeclarationIndex,
    pub(super) declaration_list: Vec<DeclarationIndex>,
    body: Option<StatementIndex>,
    pub(super) source_vectors: Option<SourceVectors>,
    starting_error_count: usize,
    entry_scope_depth: Option<usize>,
    diagnosed_prototype_declaration_list: bool,
}

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
impl FunctionDefinitionFrame {
    pub(super) fn new(head: DeclarationIndex, starting_error_count: usize) -> Self {
        Self {
            phase: FunctionDefinitionPhase::Start,
            head,
            declaration_list: Vec::new(),
            body: None,
            source_vectors: None,
            starting_error_count,
            entry_scope_depth: None,
            diagnosed_prototype_declaration_list: false,
        }
    }

    pub(super) fn step(
        &mut self,
        parser: &mut Parser,
        context: &mut Context,
        token: Option<Token>,
        returned: Option<ParseValue>,
    ) -> ParseAction {
        match self.phase {
            | FunctionDefinitionPhase::Start => {
                debug_assert!(returned.is_none());
                let declaration = parser.syntax.declarations[self.head.0 as usize];
                let declarator = parser
                    .declaration_head_declarator(self.head)
                    .expect("a function definition has one uninitialized declarator");
                self.source_vectors = Some(declaration.source_vectors);
                self.entry_scope_depth = Some(parser.scopes.depth());
                parser.scopes.enter_scope(ScopeKind::Function);
                parser.label_scopes.push(LabelScope::default());

                if let Some(suffix) = parser.function_suffix(declarator) {
                    match suffix {
                        | DirectDeclarator::Function { parameter_list, .. } => {
                            if parameter_list.length() == 0 {
                                self.phase = FunctionDefinitionPhase::DeclarationOrBody;
                                return ParseAction::Reprocess;
                            }
                            let start = parameter_list.start_index() as usize;
                            let end = start + parameter_list.length() as usize;
                            let mut names = Vec::new();
                            for parameter in &parser.syntax.parameter_declarations[start..end] {
                                parser.collect_type_specifier_bindings(
                                    parameter.declaration_specifiers.type_specifiers,
                                    &mut names,
                                );
                                if let Some(name) = parameter
                                    .declarator
                                    .and_then(|declarator| parser.declarator_identifier(declarator))
                                    .map(|identifier| identifier.name)
                                {
                                    names.push(name);
                                }
                            }
                            for name in names {
                                parser.scopes.publish(name, NameClass::Ordinary);
                            }
                        },
                        | DirectDeclarator::KAndRStyleFunction { parameters } => {
                            if parameters.length() == 0 {
                                self.phase = FunctionDefinitionPhase::DeclarationOrBody;
                                return ParseAction::Reprocess;
                            }
                            let start = parameters.start_index() as usize;
                            let end = start + parameters.length() as usize;
                            let names = parser.syntax.identifiers[start..end]
                                .iter()
                                .map(|identifier| identifier.name)
                                .collect::<Vec<_>>();
                            for name in names {
                                parser.scopes.publish(name, NameClass::Ordinary);
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
                        parser.hard_error_count,
                        true,
                    )))
                } else if token.is_some_and(|token| parser.declaration_starter(token)) {
                    let has_parameter_type_list = parser
                        .declaration_head_declarator(self.head)
                        .and_then(|declarator| parser.function_suffix(declarator))
                        .is_some_and(|suffix| matches!(suffix, DirectDeclarator::Function { .. }));
                    if has_parameter_type_list && !self.diagnosed_prototype_declaration_list {
                        self.diagnosed_prototype_declaration_list = true;
                        parser.report(
                            context,
                            ParserErrorType::DeclarationListAfterParameterTypeList,
                            token,
                        );
                    }
                    self.phase = FunctionDefinitionPhase::AwaitDeclaration;
                    ParseAction::Push(ParseFrame::Declaration(DeclarationFrame::new(
                        parser.syntax.init_declarators.len().to_u32(),
                        DeclarationContext::OldStyleParameter,
                        parser.hard_error_count,
                    )))
                } else {
                    parser.report(
                        context,
                        ParserErrorType::ExpectedFunctionBody(token.map(|token| token.kind)),
                        token,
                    );
                    // A head that is not a function declarator only became a
                    // definition because a declaration followed it; the usual
                    // cause is a missing `;` after it.
                    if let Some(declarator) = parser.declaration_head_declarator(self.head)
                        && parser.function_suffix(declarator).is_none()
                    {
                        parser.suggest_semicolon_after(context, declarator.source_vectors);
                    }
                    if token.is_none() {
                        let body_source = context.create_source_vectors(
                            parser.position(context),
                            parser.source_file_index(),
                            0,
                        );
                        let body = parser.syntax.statements.len().to_u32();
                        parser.push_syntax(
                            |syntax| &mut syntax.statements,
                            Statement {
                                kind:           StatementType::Compound {
                                    items: SyntaxList::empty(parser.syntax_id),
                                },
                                source_vectors: body_source,
                                recovered:      true,
                            },
                        );
                        self.body = Some(StatementIndex(body, parser.syntax_id));
                        self.phase = FunctionDefinitionPhase::Finish;
                        ParseAction::Reprocess
                    } else {
                        self.phase = FunctionDefinitionPhase::AwaitBody;
                        ParseAction::Push(ParseFrame::CompoundStatement(
                            CompoundStatementFrame::new(parser.hard_error_count, true),
                        ))
                    }
                }
            },
            | FunctionDefinitionPhase::AwaitDeclaration => {
                let Some(ParseValue::Declaration(declaration)) = returned else {
                    panic!("old-style declaration returned an unexpected value: {returned:?}");
                };
                let source = parser.syntax.declarations[declaration.0 as usize].source_vectors;
                self.source_vectors = Some(
                    self.source_vectors
                        .map_or(source, |existing| context.merge_vectors(existing, source)),
                );
                self.declaration_list.push(declaration);
                self.phase = FunctionDefinitionPhase::DeclarationOrBody;
                ParseAction::Reprocess
            },
            | FunctionDefinitionPhase::AwaitBody => {
                let Some(ParseValue::CompoundStatement(body)) = returned else {
                    panic!("function body returned an unexpected value: {returned:?}");
                };
                let source = parser.statement_source(body);
                self.source_vectors = Some(
                    self.source_vectors
                        .map_or(source, |existing| context.merge_vectors(existing, source)),
                );
                self.body = Some(body);
                self.phase = FunctionDefinitionPhase::Finish;
                ParseAction::Reprocess
            },
            | FunctionDefinitionPhase::Finish => {
                debug_assert!(returned.is_none());
                let head = parser.syntax.declarations[self.head.0 as usize];
                let declarator = parser
                    .declaration_head_declarator(self.head)
                    .expect("function definition head remains available");
                let declaration_start = parser.syntax.declaration_indices.len().to_u32();
                parser.append_syntax(
                    |syntax| &mut syntax.declaration_indices,
                    &mut self.declaration_list,
                );
                let recovered = parser.hard_error_count > self.starting_error_count;
                let index = parser.syntax.function_definitions.len().to_u32();
                parser.push_syntax(
                    |syntax| &mut syntax.function_definitions,
                    FunctionDefinition {
                        declaration_specifiers: head.declaration_specifiers,
                        declarator,
                        declaration_list: SyntaxList::new(
                            parser.syntax_id,
                            declaration_start,
                            parser.syntax.declaration_indices.len().to_u32(),
                        ),
                        body: self.body.expect("function definition has a body node"),
                        source_vectors: self.source_vectors.unwrap_or_default(),
                        recovered,
                    },
                );
                parser.scopes.restore_depth(
                    self.entry_scope_depth
                        .expect("function definition entered function scope"),
                );
                drop(
                    parser
                        .label_scopes
                        .pop()
                        .expect("function definition owns a label namespace"),
                );
                ParseAction::Reduce(ParseValue::FunctionDefinition(FunctionDefinitionIndex(
                    index,
                    parser.syntax_id,
                )))
            },
        }
    }
}
