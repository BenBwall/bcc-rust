//! Type-name frame used by casts, `sizeof`, and compound literals.
//!
//! Translation phase 7 syntax analysis (§5.1.1.2, p. 10; PDF p. 22) of
//! `type-name` (C99: §6.7.6 paragraph 1, p. 122; PDF p. 134; §A.2.2,
//! p. 414; PDF p. 426): a `specifier-qualifier-list` followed by an optional
//! `abstract-declarator`. A type name is a declaration that omits the
//! identifier (paragraph 2, p. 122; PDF p. 134). Its users are casts
//! (§6.5.4, p. 81; PDF p. 93), `sizeof` (§6.5.3.4, p. 80; PDF p. 92), and
//! compound literals (§6.5.2.5, p. 75; PDF p. 87). The type it names is left
//! to semantic analysis.

use std::fmt::Debug;

use super::{
    declaration_specifiers::{
        DeclarationSpecifiersFrame,
        SpecifierMode,
    },
    declarator::{
        DeclaratorFrame,
        DeclaratorMode,
    },
};
use crate::translation_phases::{
    parsing::{
        Parser,
        declaration_syntax::{
            DeclarationSpecifiers,
            Declarator,
            TypeName,
        },
        machine::{
            ParseAction,
            ParseFrame,
            ParseValue,
            unexpected_return,
        },
    },
    preprocessing::{
        OperatorTokenType,
        Token,
        TokenType,
    },
};

impl<'tu, 'p> TypeNameFrame<'tu> {
    #[expect(
        clippy::missing_assert_message,
        reason = "Frame-state debug assertions are local transition invariants."
    )]
    pub(in crate::translation_phases::parsing) fn step(
        &mut self,
        parser: &mut Parser<'_, 'tu, 'p>,
        token: Option<Token>,
        returned: Option<ParseValue<'tu>>,
    ) -> ParseAction<'tu, 'p> {
        match self.phase {
            | TypeNamePhase::Start => {
                debug_assert!(returned.is_none());
                self.phase = TypeNamePhase::AwaitSpecifiers;
                ParseAction::Push(ParseFrame::DeclarationSpecifiers(
                    DeclarationSpecifiersFrame::new(if self.compound_literal {
                        SpecifierMode::CompoundLiteral
                    } else {
                        SpecifierMode::TypeName
                    }),
                ))
            },
            | TypeNamePhase::AwaitSpecifiers => {
                let Some(ParseValue::DeclarationSpecifiers(specifiers)) = returned else {
                    unexpected_return!(
                        "type-name specifiers returned an unexpected value: {returned:?}"
                    );
                };
                self.declaration_specifiers = Some(specifiers);
                if token.is_some_and(|token| is_abstract_declarator_starter(token.kind)) {
                    self.phase = TypeNamePhase::AwaitDeclarator;
                    ParseAction::Push(ParseFrame::Declarator(DeclaratorFrame::new(
                        parser.arena,
                        DeclaratorMode::Abstract,
                    )))
                } else {
                    self.phase = TypeNamePhase::Finish(None);
                    ParseAction::Continue
                }
            },
            | TypeNamePhase::AwaitDeclarator => {
                let Some(ParseValue::Declarator(declarator)) = returned else {
                    unexpected_return!(
                        "type-name declarator returned an unexpected value: {returned:?}"
                    );
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
                        parser.context.merge_vectors(
                            declaration_specifiers.source_vectors,
                            declarator.source_vectors,
                        )
                    });
                let type_name = parser.alloc_syntax(TypeName {
                    declaration_specifiers,
                    declarator,
                    source_vectors,
                    recovered: parser.hard_error_count > self.starting_error_count,
                });
                ParseAction::Reduce(ParseValue::TypeName(type_name))
            },
        }
    }
}

/// State transitions for [`TypeNameFrame`].
///
/// C99: §6.7.6 paragraph 1, p. 122; PDF p. 134.
#[derive(Debug, Clone, Copy)]
enum TypeNamePhase<'tu> {
    Start,
    AwaitSpecifiers,
    AwaitDeclarator,
    Finish(Option<Declarator<'tu>>),
}

/// Parses one type name.
///
/// C99: §6.7.6 paragraph 1, p. 122; PDF p. 134.
#[derive(Debug, Clone, Copy)]
pub(in crate::translation_phases::parsing) struct TypeNameFrame<'tu> {
    phase:                  TypeNamePhase<'tu>,
    declaration_specifiers: Option<DeclarationSpecifiers<'tu>>,
    starting_error_count:   usize,
    compound_literal:       bool,
}

impl TypeNameFrame<'_> {
    pub(in crate::translation_phases::parsing) fn new(starting_error_count: usize) -> Self {
        Self {
            phase: TypeNamePhase::Start,
            declaration_specifiers: None,
            starting_error_count,
            compound_literal: false,
        }
    }

    pub(in crate::translation_phases::parsing) fn with_storage(mut self) -> Self {
        self.compound_literal = true;
        self
    }
}

/// Whether `token` can begin an `abstract-declarator`: `pointer`, `(`, or
/// `[`.
///
/// C99: §6.7.6 paragraph 1, p. 122; PDF p. 134.
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
