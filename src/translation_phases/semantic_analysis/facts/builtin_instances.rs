//! Checked builtin operands are retained once beside M3 call-site facts.
//! C99: §6.5.2.2p7, p. 72; PDF p. 84; §7.15.1, pp. 249-251;
//! PDF pp. 261-263. Scalar query/effects rules are GNU extensions.

use crate::{
    builtins::{
        ArgumentEvaluation,
        ArgumentRule,
        BuiltinDescriptor,
        BuiltinEffectClass,
        BuiltinId,
        BuiltinOperation,
        ConstantConstraint,
        ConversionRule,
    },
    translation_phases::{
        parsing::{
            Builtin,
            OffsetMember,
            SyntaxOperand,
            syntax::Expression,
        },
        semantic_analysis::{
            Analyzer,
            ExpressionInfo,
            Floating,
            Integer,
            SemanticTranslationUnit,
            TypeId,
            TypeKind,
        },
    },
    util::bump::ArenaVec,
};

/// Arena-owned result of the language checker, independent of lowering state.
#[derive(Clone, Copy, Debug)]
pub(crate) struct BuiltinCallFacts<'tu> {
    pub(crate) descriptor:     BuiltinId,
    pub(crate) operation:      BuiltinOperation,
    pub(crate) operands:       &'tu [BuiltinOperand<'tu>],
    pub(crate) result_type:    TypeId,
    pub(crate) operation_type: Option<TypeId>,
    pub(crate) integer:        Option<Integer>,
    pub(crate) effects:        BuiltinEffectClass,
}

/// Storage uses distinguish a cursor's address from an adjusted list pointer.
/// Type operands have no expression; no synthetic syntax nodes are allocated.
#[derive(Clone, Copy, Debug)]
pub(crate) struct BuiltinOperand<'tu> {
    pub(crate) expression:      Option<&'tu Expression<'tu>>,
    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "M4 retains normalized operand types and converted constants for M6 \
                      consumers."
        )
    )]
    pub(crate) ty:              TypeId,
    pub(crate) evaluation:      ArgumentEvaluation,
    pub(crate) storage_binding: Option<usize>,
    pub(crate) integer:         Option<Integer>,
    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "M4 retains normalized operand types and converted constants for M6 \
                      consumers."
        )
    )]
    pub(crate) floating:        Option<Floating>,
    pub(crate) bit_field:       Option<u32>,
}

impl<'tu> Analyzer<'_, 'tu, '_> {
    pub(in crate::translation_phases::semantic_analysis) fn checked_named_builtin(
        &mut self,
        info: ExpressionInfo<'tu>,
        descriptor: &BuiltinDescriptor,
        arguments: &[&'tu Expression<'tu>],
    ) -> &'tu BuiltinCallFacts<'tu> {
        let mut operands = ArenaVec::new_in(self.types.tu);
        for (index, &expression) in arguments.iter().enumerate() {
            let rule = descriptor
                .arguments
                .get(index)
                .copied()
                .unwrap_or(ArgumentRule::value(ConversionRule::Value));
            let rule = if let BuiltinOperation::Atomic(atomic) = descriptor.operation
                && index >= atomic.count
            {
                ArgumentRule::evaluation(ArgumentEvaluation::Unevaluated)
            } else {
                rule
            };
            let mut operand = self.checked_operand(expression, rule);
            if index == 0
                && matches!(
                    (
                        descriptor.operation,
                        self.function_extensions.object_size(info.expression)
                    ),
                    (
                        BuiltinOperation::ObjectSize(true),
                        Some(super::super::ObjectSize::Remainder { .. })
                    ) | (
                        BuiltinOperation::ObjectSize(_),
                        Some(super::super::ObjectSize::Deferred { .. })
                    )
                )
            {
                operand.evaluation = ArgumentEvaluation::Value;
            }
            operands.push(operand);
        }
        self.builtin_instance(info, descriptor, operands.leak())
    }

    pub(in crate::translation_phases::semantic_analysis) fn checked_keyword_builtin(
        &mut self,
        info: ExpressionInfo<'tu>,
        descriptor: &BuiltinDescriptor,
        syntax: &'tu Builtin<'tu>,
    ) -> &'tu BuiltinCallFacts<'tu> {
        let mut operands = ArenaVec::new_in(self.types.tu);
        for (index, &operand) in syntax.operands.iter().enumerate() {
            let rule = descriptor
                .arguments
                .get(index)
                .copied()
                .unwrap_or(ArgumentRule::evaluation(ArgumentEvaluation::Unevaluated));
            match operand {
                | SyntaxOperand::Expression(expression) =>
                    operands.push(self.checked_operand(expression, rule)),
                | SyntaxOperand::Type(name) => operands.push(BuiltinOperand {
                    expression:      None,
                    ty:              self
                        .resolved_type_names
                        .get(&super::super::type_name_identity(name))
                        .copied()
                        .unwrap_or_else(|| self.types.unknown()),
                    evaluation:      ArgumentEvaluation::Type,
                    storage_binding: None,
                    integer:         None,
                    floating:        None,
                    bit_field:       None,
                }),
            }
        }
        for &member in syntax.members {
            if let OffsetMember::Index(expression) = member {
                operands.push(
                    self.checked_operand(expression, ArgumentRule::value(ConversionRule::Value)),
                );
            }
        }
        self.builtin_instance(info, descriptor, operands.leak())
    }

    fn checked_operand(
        &mut self,
        expression: &'tu Expression<'tu>,
        rule: ArgumentRule,
    ) -> BuiltinOperand<'tu> {
        let info = self.expression_info(expression);
        let ty = self.resolved.argument_type(expression, info.ty);
        let mut evaluation = rule.evaluation;
        if evaluation == ArgumentEvaluation::StorageAddress
            && !self.types.target.va_list_is_pointer
            && matches!(self.types.nodes[info.ty.index], TypeKind::Pointer(_))
        {
            evaluation = ArgumentEvaluation::AdjustedPointer;
        }
        let storage_binding = if matches!(
            evaluation,
            ArgumentEvaluation::StorageAddress | ArgumentEvaluation::AdjustedPointer
        ) {
            self.designation(expression).binding
        } else {
            None
        };
        let integer = info.integer.map(|value| {
            if matches!(rule.constraint, ConstantConstraint::OriginalIntegerRange(_)) {
                return value;
            }
            self.integer_type(ty)
                .map_or(value, |(bits, signed)| value.cast(bits, signed))
        });
        let floating = self.converted_floating(info, ty);
        BuiltinOperand {
            expression: Some(expression),
            ty,
            evaluation,
            storage_binding,
            integer,
            floating,
            bit_field: info.bit_field,
        }
    }

    fn builtin_instance(
        &self,
        info: ExpressionInfo<'tu>,
        descriptor: &BuiltinDescriptor,
        operands: &'tu [BuiltinOperand<'tu>],
    ) -> &'tu BuiltinCallFacts<'tu> {
        self.types.tu.alloc(BuiltinCallFacts {
            descriptor: descriptor.id,
            operation: descriptor.operation,
            effects: descriptor.effects,
            operands,
            result_type: info.ty,
            operation_type: info.operation_type,
            // Function code emits one for a query whose constant-context
            // value is zero ([`ExpressionInfo::runtime_query`]).
            integer: if info.runtime_query
                && matches!(descriptor.operation, BuiltinOperation::Constant)
            {
                Some(Integer::int(1))
            } else {
                info.integer
            },
        })
    }
}

impl<'tu> SemanticTranslationUnit<'tu> {
    pub(crate) fn builtin_call_facts(
        &self,
        expression: &Expression<'tu>,
    ) -> Option<&'tu BuiltinCallFacts<'tu>> {
        match self.call_facts(expression)?.callee {
            | super::resolved::Callee::Builtin { instance, .. } => instance,
            | _ => None,
        }
    }
}
