//! Recursive type-name grammar used by casts, `sizeof`, and compound
//! literals.
//!
//! Translation phase 7 syntax analysis (§5.1.1.2, p. 10; PDF p. 22) of
//! `type-name` (C99: §6.7.6 paragraph 1, p. 122; PDF p. 134; §A.2.2,
//! p. 414; PDF p. 426): a `specifier-qualifier-list` followed by an optional
//! `abstract-declarator`. A type name is a declaration that omits the
//! identifier (paragraph 2, p. 122; PDF p. 134). Its users are casts
//! (§6.5.4, p. 81; PDF p. 93), `sizeof` (§6.5.3.4, p. 80; PDF p. 92), and
//! compound literals (§6.5.2.5, p. 75; PDF p. 87). The type it names is left
//! to semantic analysis.

use crate::translation_phases::{
    parsing::{
        Parser,
        declaration_syntax::TypeName,
        grammar::{
            ProductionKind,
            declaration_specifiers::SpecifierMode,
            declarators::DeclaratorMode,
        },
    },
    preprocessing::{
        OperatorTokenType,
        TokenType,
    },
};

impl<'tu> Parser<'_, 'tu, '_> {
    /// Parses one type name. `compound_literal` selects the specifier mode
    /// that admits a compound literal's storage-class specifiers; the type
    /// name is recovered when hard errors were reported after
    /// `starting_error_count`.
    ///
    /// C99: §6.7.6 paragraph 1, p. 122; PDF p. 134.
    #[inline(never)]
    pub(super) fn parse_type_name(
        &mut self,
        starting_error_count: usize,
        compound_literal: bool,
    ) -> Option<&'tu TypeName<'tu>> {
        let entry = self.enter_production(ProductionKind::TypeName)?;
        let result = self.parse_type_name_body(starting_error_count, compound_literal);
        self.leave_production(entry);
        result
    }

    fn parse_type_name_body(
        &mut self,
        starting_error_count: usize,
        compound_literal: bool,
    ) -> Option<&'tu TypeName<'tu>> {
        let mode = if compound_literal {
            SpecifierMode::CompoundLiteral
        } else {
            SpecifierMode::TypeName
        };
        let declaration_specifiers = self.parse_declaration_specifiers(mode, true)?;
        let declarator = if self
            .cursor
            .current()
            .is_some_and(|token| is_abstract_declarator_starter(token.kind))
        {
            self.parse_declarator(DeclaratorMode::Abstract)?
        } else {
            None
        };
        let source_vectors =
            declarator.map_or(declaration_specifiers.source_vectors, |declarator| {
                self.context.merge_vectors(
                    declaration_specifiers.source_vectors,
                    declarator.source_vectors,
                )
            });
        Some(self.alloc_syntax(TypeName {
            declaration_specifiers,
            declarator,
            source_vectors,
            recovered: self.hard_error_count > starting_error_count,
        }))
    }
}

/// Whether `token` can begin an `abstract-declarator`: `pointer`, `(`, or
/// `[`.
///
/// C99: §6.7.6 paragraph 1, p. 122; PDF p. 134.
pub(in crate::translation_phases::parsing) fn is_abstract_declarator_starter(
    token: TokenType,
) -> bool {
    matches!(
        token,
        TokenType::Operator(
            OperatorTokenType::Asterisk
                | OperatorTokenType::OpeningParenthesis
                | OperatorTokenType::OpeningSquareBracket
        )
    )
}
