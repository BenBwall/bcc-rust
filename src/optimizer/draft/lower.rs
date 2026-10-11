//! Writing a draft back as a body.

use super::{
    Constant,
    Draft,
    Edge,
    Term,
};
use crate::{
    ir::{
        Bits64,
        Block,
        Entity,
        FuncId,
        FunctionBuilder,
        Inst,
        InstData,
        Module,
        Opcode,
        Type,
        Value,
    },
    util::bump::ArenaVec,
};

/// How the original blocks and values are named in the new body.
struct Numbering<'s> {
    blocks: ArenaVec<'s, Option<Block>>,
    values: ArenaVec<'s, Option<Value>>,
}

impl<'s> Draft<'_, 's> {
    /// Makes the draft the body of `func`, replacing the body it has. Live
    /// blocks keep their layout order and are renumbered without gaps; so are
    /// the values, in the order the textual form lists them: each block's
    /// parameters, then its instructions' results. Stack slots keep their
    /// numbers.
    pub(in crate::optimizer) fn lower(&self, module: &mut Module<'_>, func: FuncId) {
        let body = self.body;
        let mut builder = FunctionBuilder::new(module, func);
        for (_, slot) in body.stack_slots() {
            _ = builder.create_stack_slot(slot.size, slot.align);
        }

        let mut numbering = Numbering {
            blocks: ArenaVec::with_capacity_in(self.blocks.len(), self.scratch),
            values: ArenaVec::with_capacity_in(self.value_count(), self.scratch),
        };
        numbering.blocks.resize(self.blocks.len(), None);
        numbering.values.resize(self.value_count(), None);
        for block in self.alive_blocks() {
            numbering.blocks[block.index()] = Some(builder.create_block());
        }
        // Reserve every value first, in listing order, so that an operand
        // defined in a later block (one the draft's layout puts after its
        // user) already has its number.
        for block in self.alive_blocks() {
            for &param in &self.block(block).params {
                numbering.values[param.index()] = Some(builder.reserve_value());
            }
            for &inst in &self.block(block).insts {
                if !self.is_removed(inst)
                    && let Some(result) = body.inst_result(inst)
                {
                    numbering.values[result.index()] = Some(builder.reserve_value());
                }
            }
        }
        for block in self.alive_blocks() {
            let new_block = numbering.block(block);
            for &param in &self.block(block).params {
                builder.define_block_param(
                    new_block,
                    numbering.values[param.index()].expect("reserved above"),
                    self.value_type(param),
                );
            }
        }
        for block in self.alive_blocks() {
            builder.switch_to_block(numbering.block(block));
            let data = self.block(block);
            for &inst in &data.insts {
                if !self.is_removed(inst) {
                    self.emit_inst(&mut builder, &numbering, inst);
                }
            }
            self.emit_terminator(&mut builder, &numbering, &data.term);
        }
        builder.finish();
    }

    /// Appends `inst` with its operands renumbered.
    fn emit_inst(
        &self,
        builder: &mut FunctionBuilder<'_, '_>,
        numbering: &Numbering<'_>,
        inst: Inst,
    ) {
        let body = self.body;
        let map = |value: Value| numbering.value(self.resolve(value));
        let result = body.inst_result(inst).map(|result| numbering.value(result));
        let original = *self.inst(inst);
        let data = if let Some(constant) = self.folded[inst.index()] {
            match constant {
                | Constant::Int(ty, bits) => constant_record(builder, Opcode::Iconst, ty, bits),
                | Constant::Poison(ty) => InstData::Nullary {
                    opcode: Opcode::Poison,
                    ty,
                },
            }
        } else {
            match original {
                | InstData::Binary {
                    opcode,
                    ty,
                    flags,
                    args: [a, b],
                } => InstData::Binary {
                    opcode,
                    ty,
                    flags,
                    args: [map(a), map(b)],
                },
                | InstData::Unary { opcode, ty, arg } => InstData::Unary {
                    opcode,
                    ty,
                    arg: map(arg),
                },
                | InstData::IntCompare {
                    cond,
                    ty,
                    args: [a, b],
                } => InstData::IntCompare {
                    cond,
                    ty,
                    args: [map(a), map(b)],
                },
                | InstData::FloatCompare {
                    cond,
                    ty,
                    args: [a, b],
                } => InstData::FloatCompare {
                    cond,
                    ty,
                    args: [map(a), map(b)],
                },
                | InstData::Select {
                    ty,
                    args: [cond, a, b],
                } => InstData::Select {
                    ty,
                    args: [map(cond), map(a), map(b)],
                },
                | InstData::Const { opcode, ty, .. } | InstData::WideConst { opcode, ty, .. } =>
                    constant_record(
                        builder,
                        opcode,
                        ty,
                        body.const_bits(&original).unwrap_or_default(),
                    ),
                | InstData::Nullary { .. }
                | InstData::StackAddr { .. }
                | InstData::GlobalAddr { .. }
                | InstData::FuncAddr { .. } => original,
                | InstData::Load {
                    ty,
                    flags,
                    align,
                    tag,
                    addr,
                } => InstData::Load {
                    ty,
                    flags,
                    align,
                    tag,
                    addr: map(addr),
                },
                | InstData::Store {
                    ty,
                    flags,
                    align,
                    tag,
                    args: [value, addr],
                } => InstData::Store {
                    ty,
                    flags,
                    align,
                    tag,
                    args: [map(value), map(addr)],
                },
                | InstData::MemoryRange {
                    opcode,
                    flags,
                    align,
                    args: [a, b, c],
                } => InstData::MemoryRange {
                    opcode,
                    flags,
                    align,
                    args: [map(a), map(b), map(c)],
                },
                | InstData::Call { func, args } => {
                    let args = self.mapped(body.value_list(args), &map);
                    InstData::Call {
                        func,
                        args: builder.value_list(&args),
                    }
                },
                | InstData::CallIndirect { sig, args } => {
                    let args = self.mapped(body.value_list(args), &map);
                    InstData::CallIndirect {
                        sig,
                        args: builder.value_list(&args),
                    }
                },
                | InstData::Jump { .. }
                | InstData::Brif { .. }
                | InstData::Switch { .. }
                | InstData::Return { .. }
                | InstData::Unreachable => unreachable!("terminators are not in a block's list"),
            }
        };
        _ = builder.insert_with_result(data, result);
    }

    /// Appends a block's terminator with its edges renumbered.
    fn emit_terminator(
        &self,
        builder: &mut FunctionBuilder<'_, '_>,
        numbering: &Numbering<'_>,
        term: &Term<'_>,
    ) {
        let map = |value: Value| numbering.value(self.resolve(value));
        let edge_args = |edge: &Edge<'_>| self.mapped(&edge.args, &map);
        match term {
            | Term::Jump(edge) => builder.jump(numbering.block(edge.target), &edge_args(edge)),
            | Term::Brif {
                cond,
                then,
                otherwise,
            } => builder.brif(
                map(*cond),
                (numbering.block(then.target), &edge_args(then)),
                (numbering.block(otherwise.target), &edge_args(otherwise)),
            ),
            | Term::Switch {
                value,
                default,
                cases,
            } => {
                let mut case_args: ArenaVec<'s, &'s [Value]> =
                    ArenaVec::with_capacity_in(cases.len(), self.scratch);
                for (_, edge) in cases {
                    case_args.push(
                        &*self
                            .scratch
                            .alloc_slice_fill_iter(edge.args.iter().map(|&arg| map(arg))),
                    );
                }
                let mut lowered = ArenaVec::with_capacity_in(cases.len(), self.scratch);
                for ((bits, edge), &args) in cases.iter().zip(&case_args) {
                    lowered.push((*bits as i128, numbering.block(edge.target), args));
                }
                builder.switch(
                    map(*value),
                    (numbering.block(default.target), &edge_args(default)),
                    &lowered,
                );
            },
            | Term::Return(value) => builder.ret(value.map(map)),
            | Term::Unreachable => builder.unreachable(),
        }
    }

    /// `values` after replacement and renumbering.
    fn mapped(&self, values: &[Value], map: &impl Fn(Value) -> Value) -> ArenaVec<'s, Value> {
        let mut mapped = ArenaVec::with_capacity_in(values.len(), self.scratch);
        mapped.extend(values.iter().map(|&value| map(value)));
        mapped
    }
}

impl Numbering<'_> {
    fn block(&self, block: Block) -> Block {
        self.blocks[block.index()].expect("an edge leads to a live block")
    }

    fn value(&self, value: Value) -> Value {
        self.values[value.index()].expect("an operand is defined by a live instruction or block")
    }
}

/// The record of an integer or floating constant, with bits wider than 64 in
/// the constant pool.
fn constant_record(
    builder: &mut FunctionBuilder<'_, '_>,
    opcode: Opcode,
    ty: Type,
    bits: u128,
) -> InstData {
    match u64::try_from(bits) {
        | Ok(narrow) if ty.bits() <= 64 => InstData::Const {
            opcode,
            ty,
            bits: Bits64::new(narrow),
        },
        | _ => InstData::WideConst {
            opcode,
            ty,
            constant: builder.wide_constant(bits),
        },
    }
}
