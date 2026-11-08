//! The checks of values: a `_Bool` read with a value other than 0 or 1
//! (`bool`), a floating value converted to an integer type that cannot hold
//! it (`float-cast-overflow`), a variable length array bound that is not
//! positive (`vla-bound`), zero passed to `__builtin_clz` or `__builtin_ctz`
//! (`builtin`), and `__builtin_unreachable` reached (`unreachable`).

use super::{
    super::FunctionLowerer,
    Field,
    Handler,
    Parent,
    Style,
    sites::operator_site,
};
use crate::{
    float_parsing::bits as floats,
    ir::{
        FloatCC,
        InstFlags,
        IntCC,
        Opcode,
        Type,
        Value,
    },
    target::sanitizers::UbChecks,
    translation_phases::{
        SourceVectors,
        parsing::syntax::{
            Expression,
            ExpressionType,
        },
        semantic_analysis::{
            BoundEvaluationSite,
            TypeId,
        },
    },
};

impl<'tu> FunctionLowerer<'_, '_, 'tu, '_, '_> {
    /// Checks the `_Bool` `value` read from memory as the value of
    /// `expression`: a byte other than 0 or 1 is not a value of the type.
    /// C99: §6.2.5 paragraph 2, p. 33; PDF p. 45; §6.2.6.1 paragraph 5,
    /// p. 37; PDF p. 49.
    pub(in crate::lowering) fn check_bool(
        &mut self,
        expression: &'tu Expression<'tu>,
        value: Value,
        ty: TypeId,
    ) {
        if !self.checks(UbChecks::BOOL) {
            return;
        }
        let one = self.iconst(Type::I8, 1);
        let failed = self.icmp(IntCC::Ugt, value, one);
        let site = self.report_site(expression);
        self.check_undefined(UbChecks::BOOL, failed, site, |this| {
            let fields = [Field::Type(
                this.type_descriptor(ty.unqualified(), Style::Plain),
            )];
            let operands = [this.value_handle(value)];
            Handler::LoadInvalidValue.report(this.scratch, &fields, &operands)
        });
    }

    /// Checks the conversion of the floating `value` of C type `from` to the
    /// integer type `to` of `bits` bits: its integer part must fit, so the
    /// value must lie strictly between the first values below and above the
    /// type's range that the floating type holds, as Clang compares it. A
    /// NaN fails both comparisons. A conversion to `_Bool`, from a 16-bit
    /// type or to a 128-bit type is not checked.
    /// C99: §6.3.1.4 paragraph 1, p. 43; PDF p. 55.
    pub(in crate::lowering) fn check_float_cast(
        &mut self,
        value: Value,
        from: TypeId,
        to: TypeId,
        (bits, signed): (u32, bool),
        source: SourceVectors,
    ) {
        if !self.checks(UbChecks::FLOAT_CAST_OVERFLOW) || bits > 64 {
            return;
        }
        let ty = self.draft.value_type(value);
        let precision = match ty {
            | Type::F32 => 24,
            | Type::F64 => 53,
            | Type::F80 => 64,
            | Type::F128 => 113,
            | _ => return,
        };
        // The largest value below the range, rounded down into the floating
        // type, and the smallest above it, a power of two.
        let (below, above) = if signed {
            let half = 1_i128 << (bits - 1);
            let below = if bits - 1 < precision {
                -half - 1
            } else {
                -half - (1_i128 << (bits - precision))
            };
            (below, half)
        } else {
            (-1, 1_i128 << bits)
        };
        let (Some(below), Some(above)) = (
            floats::from_integer(below, ty),
            floats::from_integer(above, ty),
        ) else {
            return;
        };
        let below = self.fconst(ty, below);
        let above = self.fconst(ty, above);
        let low = self.fcmp(FloatCC::Ogt, value, below);
        let high = self.fcmp(FloatCC::Olt, value, above);
        let fits = self.binary_inst(Opcode::And, InstFlags::empty(), low, high);
        let no = self.iconst(Type::I1, 0);
        let failed = self.icmp(IntCC::Eq, fits, no);
        // Clang reports at the cast, or at the converted operand; GCC at the
        // operand's operator, or where the statement starts when the operand
        // is a name, which has no location of its own in GCC.
        let site = match self.ub.site {
            | Some(expression) if self.gcc_reports() => {
                let mut operand = expression;
                while let ExpressionType::Parenthesized { expression: inner } = operand.kind {
                    operand = inner;
                }
                match operand.kind {
                    | ExpressionType::Identifier(_) => self.ub.statement.unwrap_or(source),
                    | _ => operator_site(operand),
                }
            },
            | Some(expression) => match self.parent(expression) {
                | Some(Parent::Cast(cast)) => cast.source_vectors,
                | _ => expression.source_vectors,
            },
            | None => source,
        };
        self.check_undefined(UbChecks::FLOAT_CAST_OVERFLOW, failed, site, |this| {
            let fields = [
                Field::Type(this.type_descriptor(from.unqualified(), Style::Plain)),
                Field::Type(this.type_descriptor(to.unqualified(), Style::Plain)),
            ];
            let operands = [this.value_handle(value)];
            Handler::FloatCastOverflow.report(this.scratch, &fields, &operands)
        });
    }

    /// Checks the variable length array bound `value`, of C type `ty`, that
    /// the bound `index` evaluated: it must be positive. Clang reports it at
    /// the size expression, GCC at the declared name.
    /// C99: §6.7.5.2 paragraph 5, p. 117; PDF p. 129.
    pub(in crate::lowering) fn check_vla_bound(&mut self, index: usize, value: Value, ty: TypeId) {
        if !self.checks(UbChecks::VLA_BOUND) {
            return;
        }
        let bound = self.unit.sema.runtime_bounds[index];
        let signed = self
            .repr(ty, bound.expression.source_vectors)
            .is_ok_and(super::super::types::Repr::signed);
        let zero = self.iconst(self.draft.value_type(value), 0);
        let failed = self.icmp(if signed { IntCC::Sle } else { IntCC::Eq }, value, zero);
        let declared = match bound.site {
            | BoundEvaluationSite::Declaration(occurrence) if self.gcc_reports() => self
                .unit
                .sema
                .correspondence
                .declarations
                .get(occurrence.0)
                .and_then(|facts| facts.binding)
                .map(|binding| self.unit.sema.bindings[binding].name.source_vectors),
            | _ => None,
        };
        let site = declared.unwrap_or_else(|| self.report_site(bound.expression));
        self.check_undefined(UbChecks::VLA_BOUND, failed, site, |this| {
            let fields = [Field::Type(
                this.type_descriptor(ty.unqualified(), Style::Plain),
            )];
            let operands = [this.value_handle(value)];
            Handler::VlaBoundNotPositive.report(this.scratch, &fields, &operands)
        });
    }

    /// Checks that `value`, the operand of the `__builtin_clz` (`leading`) or
    /// `__builtin_ctz` call `call`, is not zero, for which both leave the
    /// result undefined. Clang reports it at the operand, GCC at the call.
    /// GNU extension: GCC manual, "Other Builtins".
    pub(in crate::lowering) fn check_bit_count(
        &mut self,
        call: &'tu Expression<'tu>,
        value: Value,
        leading: bool,
    ) {
        if !self.checks(UbChecks::BUILTIN) {
            return;
        }
        let zero = self.iconst(self.draft.value_type(value), 0);
        let failed = self.icmp(IntCC::Eq, value, zero);
        // GCC's call statement keeps the call's location, whatever it is
        // assigned to.
        let site = match call.kind {
            | ExpressionType::Call { arguments, .. } if !self.gcc_reports() => arguments
                .iter()
                .next()
                .map_or(call.source_vectors, |&argument| self.operand_site(argument)),
            | _ => self.call_site(call),
        };
        self.check_undefined(UbChecks::BUILTIN, failed, site, |this| {
            Handler::InvalidBuiltin.report(this.scratch, &[Field::Byte(u8::from(leading))], &[])
        });
    }

    /// Reports `__builtin_unreachable`, the call `call`, reached. The caller
    /// ends the block.
    /// GNU extension: GCC manual, "Other Builtins".
    pub(in crate::lowering) fn check_unreachable(&mut self, call: &'tu Expression<'tu>) {
        if !self.checks(UbChecks::UNREACHABLE) {
            return;
        }
        let reached = self.iconst(Type::I1, 1);
        let site = operator_site(call);
        self.check_undefined(UbChecks::UNREACHABLE, reached, site, |this| {
            Handler::BuiltinUnreachable.report(this.scratch, &[], &[])
        });
    }
}
