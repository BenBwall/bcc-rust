//! Atomic instruction emission and derived read-modify-write results.
//! Signed atomic fetch arithmetic deliberately has no NSW flag.
//! C11: §7.17.7.5p3, pp. 284-285; PDF pp. 302-303.
use super::{
    A,
    Align,
    FunctionLowerer,
    InstData,
    InstFlags,
    IntCC,
    M,
    Op,
    Opcode,
    Type,
    Value,
    ValueList,
};

impl FunctionLowerer<'_, '_, '_, '_, '_> {
    pub(in crate::lowering) fn atomic_inst(
        &mut self,
        op: A,
        ty: Type,
        args: &[Value],
        orders: [M; 2],
        align: Align,
        volatile: bool,
    ) -> Option<Value> {
        let [mut order, mut failure] = orders;
        // Dynamic orders conservatively use seq_cst. Invalid constant orders
        // retain the front end's warning but must never create invalid LLVM IR.
        if matches!(op, A::Load | A::LoadBytes) && matches!(order, M::Release | M::AcqRel)
            || matches!(op, A::Store | A::StoreBytes) && matches!(order, M::Acquire | M::AcqRel)
        {
            order = M::SeqCst;
        }
        if matches!(op, A::CompareExchange | A::CompareExchangeBytes)
            && !order.valid_failure(failure)
        {
            order = M::SeqCst;
            failure = M::SeqCst;
        }
        let data = InstData::Atomic {
            op,
            ty,
            flags: if volatile {
                InstFlags::VOLATILE
            } else {
                InstFlags::empty()
            },
            align,
            order,
            failure,
            args: ValueList::EMPTY,
        };
        let data = self.scratch.alloc(data);
        let args = self.scratch.alloc_slice_copy(args);
        self.draft
            .push(Op::Atomic(data, args), op.has_result().then_some(ty))
    }

    pub(super) fn atomic_updated(&mut self, op: A, old: Value, operand: Value) -> Value {
        let ty = self.draft.value_type(old);
        match op {
            | A::Min | A::Max | A::Umin | A::Umax => {
                let condition = match op {
                    | A::Min => IntCC::Slt,
                    | A::Max => IntCC::Sgt,
                    | A::Umin => IntCC::Ult,
                    | _ => IntCC::Ugt,
                };
                let keep = self.icmp(condition, old, operand);
                self.choose(keep, old, operand)
            },
            | A::Fmin | A::Fmax => self.atomic_float_extreme(op == A::Fmin, old, operand),
            | _ => {
                let opcode = match op {
                    | A::Add => Opcode::Iadd,
                    | A::Sub => Opcode::Isub,
                    | A::And | A::Nand => Opcode::And,
                    | A::Or => Opcode::Or,
                    | A::Xor => Opcode::Xor,
                    | A::Fadd => Opcode::Fadd,
                    | A::Fsub => Opcode::Fsub,
                    | _ => unreachable!("RMW update"),
                };
                let value = self.plain(opcode, old, operand);
                if op == A::Nand {
                    let ones = self.iconst(ty, -1);
                    self.plain(Opcode::Xor, value, ones)
                } else {
                    value
                }
            },
        }
    }

    pub(super) fn atomic_float_extreme(
        &mut self,
        minimum: bool,
        old: Value,
        operand: Value,
    ) -> Value {
        let condition = if minimum {
            crate::ir::FloatCC::Olt
        } else {
            crate::ir::FloatCC::Ogt
        };
        let keep = self.fcmp(condition, old, operand);
        let selected = self.choose(keep, old, operand);
        let operand_nan = self.fcmp(crate::ir::FloatCC::Uno, operand, operand);
        self.choose(operand_nan, old, selected)
    }
}
