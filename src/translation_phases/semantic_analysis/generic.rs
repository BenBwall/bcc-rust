//! C11 generic selection typing and selected-expression constant propagation.
//! C11: §6.5.1.1, p. 78; PDF p. 96. Children use the existing work stack.

use super::{
    Analyzer,
    ArenaVec,
    Expression,
    ExpressionInfo,
    SemanticErrorKind,
    SyntaxOperand,
    TypeKind,
};
use crate::translation_phases::parsing::GenericSelection;

impl<'tu> Analyzer<'_, 'tu, '_> {
    pub(super) fn type_generic(
        &mut self,
        e: &'tu Expression<'tu>,
        generic: &'tu GenericSelection<'tu>,
    ) -> ExpressionInfo<'tu> {
        let controlling = match generic.controlling {
            | SyntaxOperand::Expression(operand) => self.converted(self.expression_info(operand)),
            | SyntaxOperand::Type(name) => self
                .resolved_type_names
                .get(&name.source_vectors)
                .copied()
                .unwrap_or_else(|| self.types.unknown()),
        };
        if generic.recovered || self.types.unanalyzed(controlling) {
            return Self::expression_result(e, self.types.unknown());
        }
        let mut types = ArenaVec::new_in(self.scratch);
        let mut selected = None;
        let mut default = None;
        let mut invalid = false;
        for association in generic.associations {
            if let Some(name) = association.type_name {
                let ty = self
                    .resolved_type_names
                    .get(&name.source_vectors)
                    .copied()
                    .unwrap_or_else(|| self.types.unknown());
                if self.types.unanalyzed(ty) {
                    return Self::expression_result(e, self.types.unknown());
                }
                invalid |= !self.complete_object(ty)
                    || self.types.variably_modified(ty)
                    || matches!(self.types.nodes[ty.index], TypeKind::Function { .. });
                for &previous in &types {
                    invalid |= self.types.composite(previous, ty).is_some();
                }
                types.push(ty);
                if self.types.composite(controlling, ty).is_some() {
                    invalid |= selected.is_some();
                    selected = Some(association.expression);
                }
            } else {
                invalid |= default.is_some();
                default = Some(association.expression);
            }
        }
        let selected = selected.or(default);
        if invalid || selected.is_none() {
            self.error(
                SemanticErrorKind::InvalidGenericSelection,
                e.source_vectors,
                None,
                None,
            );
            return Self::expression_result(e, self.types.unknown());
        }
        selected.map_or_else(
            || Self::expression_result(e, self.types.unknown()),
            |selected| ExpressionInfo {
                expression: e,
                ..self.expression_info(selected)
            },
        )
    }
}
