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

use crate::translation_phases::parsing::{
    Parser,
    declaration_syntax::TypeName,
    frames::{
        declaration_specifiers::{
            DeclarationSpecifiersFrame,
            SpecifierMode,
        },
        declarator::DeclaratorMode,
        type_name::is_abstract_declarator_starter,
    },
    machine::{
        ParseFrame,
        ParseFrameKind,
        ParseValue,
        unexpected_return,
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
    pub(super) fn rd_type_name_syntax(
        &mut self,
        starting_error_count: usize,
        compound_literal: bool,
    ) -> Option<&'tu TypeName<'tu>> {
        let entry = self.rd_enter(ParseFrameKind::TypeName)?;
        let result = self.rd_type_name_body(starting_error_count, compound_literal);
        self.study_leave(entry);
        result
    }

    fn rd_type_name_body(
        &mut self,
        starting_error_count: usize,
        compound_literal: bool,
    ) -> Option<&'tu TypeName<'tu>> {
        let value = self.rd_child(ParseFrame::DeclarationSpecifiers(
            DeclarationSpecifiersFrame::new(if compound_literal {
                SpecifierMode::CompoundLiteral
            } else {
                SpecifierMode::TypeName
            }),
        ))?;
        let ParseValue::DeclarationSpecifiers(declaration_specifiers) = value else {
            unexpected_return!("type-name specifiers returned an unexpected value: {value:?}");
        };
        let declarator = if self
            .cursor
            .current()
            .is_some_and(|token| is_abstract_declarator_starter(token.kind))
        {
            self.rd_declarator(DeclaratorMode::Abstract)?
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
