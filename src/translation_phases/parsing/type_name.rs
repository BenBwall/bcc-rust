//! Type-name frame used by casts, `sizeof`, and compound literals.

use std::fmt::Debug;

use super::{
    Parser,
    declaration_specifiers::{
        DeclarationSpecifiersFrame,
        SpecifierMode,
    },
    declaration_syntax::{
        DeclarationSpecifiers,
        Declarator,
        TypeName,
    },
    declarator::{
        DeclaratorFrame,
        DeclaratorMode,
    },
    machine::{
        ParseAction,
        ParseFrame,
        ParseValue,
    },
    syntax::TypeNameIndex,
};
use crate::translation_phases::{
    Context,
    preprocessing::{
        OperatorTokenType,
        Token,
        TokenType,
    },
};

#[derive(Debug, Clone, Copy)]
pub(super) struct TypeNameFrame {
    phase:                  TypeNamePhase,
    declaration_specifiers: Option<DeclarationSpecifiers>,
    starting_error_count:   usize,
}

#[derive(Debug, Clone, Copy)]
pub(super) enum TypeNamePhase {
    Start,
    AwaitSpecifiers,
    AwaitDeclarator,
    Finish(Option<Declarator>),
}

impl TypeNameFrame {
    pub(super) fn new(starting_error_count: usize) -> Self {
        Self {
            phase: TypeNamePhase::Start,
            declaration_specifiers: None,
            starting_error_count,
        }
    }

    #[expect(
        clippy::missing_assert_message,
        reason = "Frame-state debug assertions are local transition invariants."
    )]
    pub(super) fn step(
        &mut self,
        parser: &mut Parser,
        context: &mut Context<'_>,
        token: Option<Token>,
        returned: Option<ParseValue>,
    ) -> ParseAction {
        match self.phase {
            | TypeNamePhase::Start => {
                debug_assert!(returned.is_none());
                self.phase = TypeNamePhase::AwaitSpecifiers;
                ParseAction::Push(ParseFrame::DeclarationSpecifiers(
                    DeclarationSpecifiersFrame::new(SpecifierMode::TypeName),
                ))
            },
            | TypeNamePhase::AwaitSpecifiers => {
                let Some(ParseValue::DeclarationSpecifiers(specifiers)) = returned else {
                    panic!("type-name specifiers returned an unexpected value: {returned:?}");
                };
                self.declaration_specifiers = Some(specifiers);
                if token.is_some_and(|token| is_abstract_declarator_starter(token.kind)) {
                    self.phase = TypeNamePhase::AwaitDeclarator;
                    ParseAction::Push(ParseFrame::Declarator(DeclaratorFrame::new(
                        DeclaratorMode::Abstract,
                    )))
                } else {
                    self.phase = TypeNamePhase::Finish(None);
                    ParseAction::Continue
                }
            },
            | TypeNamePhase::AwaitDeclarator => {
                let Some(ParseValue::Declarator(declarator)) = returned else {
                    panic!("type-name declarator returned an unexpected value: {returned:?}");
                };
                self.phase = TypeNamePhase::Finish(declarator);
                ParseAction::Continue
            },
            | TypeNamePhase::Finish(declarator) => {
                debug_assert!(returned.is_none());
                let declaration_specifiers = self
                    .declaration_specifiers
                    .expect("type name cannot finish without specifiers");
                let source_vectors =
                    declarator.map_or(declaration_specifiers.source_vectors, |declarator| {
                        context.merge_vectors(
                            declaration_specifiers.source_vectors,
                            declarator.source_vectors,
                        )
                    });
                let index = TypeNameIndex(parser.push_syntax(TypeName {
                    declaration_specifiers,
                    declarator,
                    source_vectors,
                    recovered: parser.hard_error_count > self.starting_error_count,
                }));
                ParseAction::Reduce(ParseValue::TypeName(index))
            },
        }
    }
}

fn is_abstract_declarator_starter(token: TokenType) -> bool {
    matches!(
        token,
        TokenType::Operator(
            OperatorTokenType::Asterisk
                | OperatorTokenType::OpeningParenthesis
                | OperatorTokenType::OpeningSquareBracket
        )
    )
}
