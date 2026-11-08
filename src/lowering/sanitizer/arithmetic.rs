//! The arithmetic checks: signed overflow of `+`, `-`, `*`, negation and
//! `++`/`--` (`signed-integer-overflow`), shift counts and the bits a left
//! shift moves (`shift-exponent`, `shift-base`), and division by zero or of
//! the most negative value by -1 (`integer-divide-by-zero`,
//! `signed-integer-overflow`).

use super::{
    super::{
        FunctionLowerer,
        types::Repr,
    },
    Field,
    Handler,
    Style,
};
use crate::{
    configuration::CStandard,
    ir::{
        InstFlags,
        IntCC,
        Opcode,
        Type,
        Value,
    },
    target::sanitizers::UbChecks,
    translation_phases::{
        SourceVectors,
        semantic_analysis::TypeId,
    },
};

impl<'s> FunctionLowerer<'_, '_, '_, '_, 's> {
    /// Checks the signed `iadd`, `isub` or `imul` of `a` and `b` in the C
    /// type `ty`, whose wrapped result is `result`; with `negation`, `a` is
    /// zero and the operation is `-b`. A narrow bit-precise operation is not
    /// checked, nor one `-fwrapv` defines. The library prints the operands,
    /// which GCC orders with a constant second, as its folding leaves them.
    /// C99: §6.5 paragraph 5, p. 67; PDF p. 79.
    #[expect(
        clippy::too_many_arguments,
        reason = "The operation, its operands, its result and its C type are all reported."
    )]
    pub(in crate::lowering) fn check_signed_overflow(
        &mut self,
        opcode: Opcode,
        representation: Repr,
        [a, b]: [Value; 2],
        result: Value,
        ty: TypeId,
        negation: bool,
        source: SourceVectors,
    ) {
        // `-fwrapv` defines the overflow, and both compilers then leave it.
        let check = UbChecks::SIGNED_INTEGER_OVERFLOW;
        if !self.checks(check)
            || representation.narrow().is_some()
            || self.unit.context.configuration.wrapv()
        {
            return;
        }
        let Some(overflow) = self.overflow_condition(opcode, [a, b], result) else {
            return;
        };
        let site = self.current_site(source);
        self.check_undefined(check, overflow, site, |this| {
            let descriptor = this.type_descriptor(ty.unqualified(), Style::Plain);
            let fields = [Field::Type(descriptor)];
            if negation {
                let operand = this.value_handle(b);
                return Handler::NegateOverflow.report(this.scratch, &fields, &[operand]);
            }
            let swap = this.gcc_reports()
                && opcode != Opcode::Isub
                && this.draft.constant(a).is_some()
                && this.draft.constant(b).is_none();
            let (left, right) = if swap { (b, a) } else { (a, b) };
            let operands = [this.value_handle(left), this.value_handle(right)];
            let handler = match opcode {
                | Opcode::Iadd => Handler::AddOverflow,
                | Opcode::Isub => Handler::SubOverflow,
                | _ => Handler::MulOverflow,
            };
            handler.report(this.scratch, &fields, &operands)
        });
    }

    /// Checks the shift of `value` by `count`, at the count's own width, and
    /// `narrowed`, the count converted to the value's width. `ty` and
    /// `count_ty` are the promoted operands' C types. A left shift of a signed
    /// value may not move a set bit into or past the sign bit, except where
    /// `-fwrapv` defines it, or before C99 in GCC. C99: §6.5.7 paragraphs 3-4,
    /// p. 84; PDF p. 96.
    #[expect(
        clippy::too_many_arguments,
        reason = "Both operands are reported with their C types."
    )]
    pub(in crate::lowering) fn check_shift(
        &mut self,
        left: bool,
        value: Value,
        [count, narrowed]: [Value; 2],
        ty: TypeId,
        count_ty: TypeId,
        signed: bool,
        source: SourceVectors,
    ) {
        let configuration = self.unit.context.configuration;
        let exponent = self.checks(UbChecks::SHIFT_EXPONENT);
        let base = self.checks(UbChecks::SHIFT_BASE)
            && left
            && signed
            && !configuration.wrapv()
            && !(self.gcc_reports() && configuration.standard() < CStandard::C99);
        if !exponent && !base {
            return;
        }
        // The count is checked at its own width, before the shift narrows
        // it to the value's (Clang's and GCC's `count > width - 1`).
        let width = self.draft.value_type(value).bits();
        let ty_ir = self.draft.value_type(value);
        let largest = self.iconst(ty_ir, i128::from(width - 1));
        let limit = self.iconst(self.draft.value_type(count), i128::from(width - 1));
        let in_range = self.icmp(IntCC::Ule, count, limit);
        let plain = InstFlags::empty();
        let mut failures = [None, None];
        if exponent {
            let no = self.iconst(Type::I1, 0);
            failures[0] = Some((UbChecks::SHIFT_EXPONENT, self.icmp(IntCC::Eq, in_range, no)));
        }
        if base {
            // The bits that would reach the sign bit or past it, by a count
            // kept in range where it is not.
            let rest = self.binary_inst(Opcode::Isub, plain, largest, narrowed);
            let zero = self.iconst(ty_ir, 0);
            let amount = self.choose(in_range, rest, zero);
            let moved = self.binary_inst(Opcode::Lshr, plain, value, amount);
            let lost = self.icmp(IntCC::Ne, moved, zero);
            let failed = self.binary_inst(Opcode::And, plain, in_range, lost);
            failures[1] = Some((UbChecks::SHIFT_BASE, failed));
        }
        let site = self.front_end_site(source);
        let report = |this: &mut Self| {
            let fields = [
                Field::Type(this.type_descriptor(ty.unqualified(), Style::Plain)),
                Field::Type(this.type_descriptor(count_ty.unqualified(), Style::Plain)),
            ];
            let operands = [this.value_handle(value), this.value_handle(count)];
            Handler::ShiftOutOfBounds.report(this.scratch, &fields, &operands)
        };
        self.check_together(failures, site, report);
    }

    /// Checks the integer division or remainder of `a` by `b` in the C type
    /// `ty`: by zero, and, for a signed type, of the most negative value by
    /// -1, whose quotient does not fit even where `-fwrapv` wraps the others.
    /// C99: §6.5.5 paragraphs 5-6, p. 82; PDF p. 94.
    pub(in crate::lowering) fn check_division(
        &mut self,
        [a, b]: [Value; 2],
        ty: TypeId,
        signed: bool,
        source: SourceVectors,
    ) {
        // GCC checks the quotient that does not fit under `-fwrapv` too, and
        // Clang does not.
        let zero_check = self.checks(UbChecks::INTEGER_DIVIDE_BY_ZERO);
        let overflow_check = signed
            && self.checks(UbChecks::SIGNED_INTEGER_OVERFLOW)
            && (self.gcc_reports() || !self.unit.context.configuration.wrapv());
        if !zero_check && !overflow_check {
            return;
        }
        let ty_ir = self.draft.value_type(a);
        let mut failures = [None, None];
        if zero_check {
            let zero = self.iconst(ty_ir, 0);
            failures[0] = Some((
                UbChecks::INTEGER_DIVIDE_BY_ZERO,
                self.icmp(IntCC::Eq, b, zero),
            ));
        }
        if overflow_check {
            let minimum = self.iconst(ty_ir, i128::MIN >> (128 - ty_ir.bits()));
            let minus_one = self.iconst(ty_ir, -1);
            let most_negative = self.icmp(IntCC::Eq, a, minimum);
            let by_minus_one = self.icmp(IntCC::Eq, b, minus_one);
            let failed =
                self.binary_inst(Opcode::And, InstFlags::empty(), most_negative, by_minus_one);
            failures[1] = Some((UbChecks::SIGNED_INTEGER_OVERFLOW, failed));
        }
        let site = self.front_end_site(source);
        let report = |this: &mut Self| {
            let fields = [Field::Type(
                this.type_descriptor(ty.unqualified(), Style::Plain),
            )];
            let operands = [this.value_handle(a), this.value_handle(b)];
            Handler::DivremOverflow.report(this.scratch, &fields, &operands)
        };
        self.check_together(failures, site, report);
    }

    /// Checks the failures of one operation that one handler reports: as one
    /// branch when the options treat their checks alike, as both compilers
    /// combine them, and one after the other when they do not.
    pub(super) fn check_together<const N: usize>(
        &mut self,
        failures: [Option<(UbChecks, Value)>; N],
        site: SourceVectors,
        report: impl Fn(&mut Self) -> super::Report<'s>,
    ) {
        let options = self.unit.sanitizer.options;
        let mode = |check: UbChecks| {
            (
                options.trap.contains(check),
                options.recover.contains(check),
            )
        };
        let mut failures = failures.into_iter().flatten().peekable();
        let Some(&(first, _)) = failures.peek() else {
            return;
        };
        let failures: [Option<(UbChecks, Value)>; N] = {
            let mut collected = [None; N];
            for (slot, failure) in collected.iter_mut().zip(failures) {
                *slot = Some(failure);
            }
            collected
        };
        if failures
            .iter()
            .flatten()
            .all(|&(check, _)| mode(check) == mode(first))
        {
            let mut checks = UbChecks::empty();
            let mut combined = None;
            for &(check, failed) in failures.iter().flatten() {
                checks |= check;
                combined = Some(match combined {
                    | Some(before) =>
                        self.binary_inst(Opcode::Or, InstFlags::empty(), before, failed),
                    | None => failed,
                });
            }
            if let Some(failed) = combined {
                self.check_undefined(checks, failed, site, &report);
            }
            return;
        }
        for (check, failed) in failures.into_iter().flatten() {
            self.check_undefined(check, failed, site, &report);
        }
    }
}
