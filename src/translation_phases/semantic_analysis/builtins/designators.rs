//! Intrinsic-only designators must reach a call through transparent syntax.
//! Library aliases retain their ordinary function-address contract.
//! GNU extension: GCC "Built-in Functions Provided by GCC" and "Other
//! Builtins". <https://gcc.gnu.org/onlinedocs/gcc/Library-Builtins.html>
//! <https://gcc.gnu.org/onlinedocs/gcc/Other-Builtins.html>
//! C99: §6.5.1p5, p. 69; PDF p. 81 (parentheses preserve designation).

use crate::{
    translation_phases::{
        parsing::{
            SyntaxOperand,
            syntax::{
                Expression,
                ExpressionType,
                UnaryOperator,
            },
        },
        preprocessing::KeywordTokenType,
        semantic_analysis::{
            Analyzer,
            SemanticErrorKind,
            ValueCategory,
        },
    },
    util::bump::ArenaMap,
};

impl Analyzer<'_, '_, '_> {
    /// A linear pass over retained expressions marks immediate transparent
    /// children and callees. Only the outermost non-call use is diagnosed;
    /// both operands of a compile-time selection remain legal designators.
    /// GNU extension: GCC manuals linked above; library aliases have addresses.
    pub(in crate::translation_phases::semantic_analysis) fn check_builtin_designators(&mut self) {
        let mut permitted = ArenaMap::with_hasher_in(rustc_hash::FxBuildHasher, self.scratch);
        for info in &self.expressions {
            let mut permit = |e: &Expression<'_>| {
                _ = permitted.insert(std::ptr::from_ref(e).addr(), ());
            };
            match info.expression.kind {
                | ExpressionType::Call {
                    function_expression,
                    ..
                } => permit(function_expression),
                | ExpressionType::Parenthesized { expression } => permit(expression),
                | ExpressionType::Unary {
                    operator: UnaryOperator::Extension,
                    operand_expression,
                } => permit(operand_expression),
                | ExpressionType::Generic(generic) =>
                    for association in generic.associations {
                        permit(association.expression);
                    },
                | ExpressionType::Builtin(builtin)
                    if builtin.keyword == KeywordTokenType::BuiltinChooseExpr =>
                {
                    for operand in builtin.operands.iter().skip(1) {
                        if let SyntaxOperand::Expression(e) = operand {
                            permit(e);
                        }
                    }
                },
                | _ => {},
            }
        }
        for index in 0..self.expressions.len() {
            let info = self.expressions[index];
            if info.category != ValueCategory::FunctionDesignator
                || permitted.contains_key(&std::ptr::from_ref(info.expression).addr())
            {
                continue;
            }
            let Some(binding) = info.binding.map(|id| self.bindings[id]) else {
                continue;
            };
            if binding.origin.is_intrinsic()
                && binding.origin.descriptor().is_none_or(|d| {
                    d.designator == crate::builtins::DesignatorPolicy::DirectCallOnly
                })
            {
                self.error(
                    SemanticErrorKind::BuiltinRequiresCall,
                    info.expression.source_vectors,
                    Some(binding.name.name),
                    None,
                );
            }
        }
    }
}
