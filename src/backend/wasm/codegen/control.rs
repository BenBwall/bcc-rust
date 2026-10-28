//! The stackifier's emitter: block bodies, edges with their block
//! arguments, two-way branches, and the selection of a `switch` or an
//! `indirect_br`.

use super::{
    Codegen,
    values::Repr,
};
use crate::{
    backend::wasm::{
        instr::{
            Instr,
            ValType,
            op,
        },
        low32,
        low64,
        stackify::{
            Emitter,
            Target,
        },
        u32_of,
    },
    ir::{
        Block,
        Entity,
        InstData,
        Type,
    },
};

/// The most labels a dense `switch` lists in its `br_table`, and how many
/// labels per case it may list at most, past a small allowance.
const DENSE_LIMIT: u128 = 1 << 12;

impl Emitter for Codegen<'_, '_, '_, '_, '_> {
    fn block(&mut self) {
        self.emit(Instr::Block);
    }

    fn r#loop(&mut self) {
        self.emit(Instr::Loop);
    }

    fn r#if(&mut self, from: u32) {
        let terminator = self
            .body
            .terminator(Block::new(from as usize))
            .expect("a branch ends its block");
        if let InstData::Brif { cond, .. } = *self.body.inst(terminator) {
            self.push(cond);
        }
        self.emit(Instr::If);
    }

    fn r#else(&mut self) {
        self.emit(Instr::Else);
    }

    fn end(&mut self) {
        self.emit(Instr::End);
    }

    fn body(&mut self, block: u32) {
        let block = Block::new(block as usize);
        let body = self.body;
        let insts = body.block_insts(block);
        for (index, &inst) in insts.iter().enumerate() {
            if self.body.inst(inst).is_terminator() {
                break;
            }
            if self
                .body
                .inst_result(inst)
                .is_some_and(|value| self.recomputed(value))
            {
                continue;
            }
            self.emit(Instr::Mark {
                block: block.as_u32(),
                inst:  u32_of(index),
                line:  body.inst_line(inst),
            });
            self.instruction(inst);
        }
        self.emit(Instr::Mark {
            block: block.as_u32(),
            inst:  u32::MAX,
            line:  body
                .terminator(block)
                .map_or(0, |inst| body.inst_line(inst)),
        });
    }

    fn edge(&mut self, from: u32, edge: u32, target: Target) {
        let body = self.body;
        let call = self.edge_calls[(self.edge_starts[from as usize] + edge) as usize];
        let (block, args) = body.block_call(call);
        // The operand stack makes the assignment parallel: every argument
        // is read before any parameter is written.
        for &arg in args {
            self.push(arg);
        }
        for &param in body.block_params(block).iter().rev() {
            self.set(param);
        }
        match target {
            | Target::Br(depth) => self.emit(Instr::Br(depth)),
            | Target::Inline => {},
            | Target::Dispatch { index, depth } => {
                let label = self.label_local();
                self.i32_const(index as i32);
                self.emit(Instr::LocalSet(label));
                self.emit(Instr::Br(depth));
            },
        }
    }

    fn leave(&mut self, from: u32) {
        let terminator = self
            .body
            .terminator(Block::new(from as usize))
            .expect("a leaving block has a terminator");
        match *self.body.inst(terminator) {
            | InstData::Return { value } => self.ret(value),
            | _ => self.emit(Instr::Unreachable),
        }
    }

    fn select(&mut self, from: u32, depths: &[u32]) {
        let block = Block::new(from as usize);
        let terminator = self
            .body
            .terminator(block)
            .expect("a selection ends its block");
        match *self.body.inst(terminator) {
            | InstData::Switch { value, cases, .. } => {
                let ty = self.ty(value);
                let first = self.edge_starts[block.index()];
                // Each case's edge, by the position its target and
                // arguments have among the distinct edges.
                let mut table = crate::util::bump::ArenaVec::new_in(self.arena);
                for (bits, dest) in self.body.switch_cases(cases) {
                    let call = self.body.block_call(dest);
                    let edge = (first..self.edge_starts[block.index() + 1])
                        .position(|index| {
                            self.body.block_call(self.edge_calls[index as usize]) == call
                        })
                        .expect("every case's edge is listed");
                    table.push((bits & ty.mask(), depths[edge]));
                }
                self.switch(value, ty, &table, depths[0]);
            },
            | InstData::IndirectBr { addr, .. } => {
                let edges = self.edge_starts[block.index() + 1] - self.edge_starts[block.index()];
                for edge in 0..edges {
                    let call = self.edge_calls[(self.edge_starts[from as usize] + edge) as usize];
                    let (target, _) = self.body.block_call(call);
                    let label = self.block_labels[target.index()];
                    self.push(addr);
                    self.i32_const(label as i32 + 1);
                    self.num(op::I32_EQ);
                    self.emit(Instr::BrIf(depths[edge as usize]));
                }
                // Any other address is undefined behaviour.
                self.emit(Instr::Unreachable);
            },
            | _ => self.refuse("a selection that is neither a switch nor an indirect branch"),
        }
    }

    fn dispatch(&mut self, count: u32) {
        let label = self.label_local();
        self.emit(Instr::LocalGet(label));
        let start = u32_of(self.labels.len());
        self.labels.extend(0..count);
        self.emit(Instr::BrTable {
            start,
            count,
            default: count.saturating_sub(1),
        });
    }
}

impl Codegen<'_, '_, '_, '_, '_> {
    /// The dispatch loop's label local, made on first use.
    fn label_local(&mut self) -> u32 {
        if let Some(label) = self.label {
            return label;
        }
        let label = self.local(ValType::I32, "label");
        self.label = Some(label);
        label
    }

    /// A `switch` on `value`: a `br_table` when the cases are dense, else a
    /// chain of comparisons. `table` pairs each case value with the depth of
    /// its edge's block.
    fn switch(&mut self, value: crate::ir::Value, ty: Type, table: &[(u128, u32)], default: u32) {
        let repr = self.repr(ty);
        let (min, max) = table.iter().fold((u128::MAX, 0), |(min, max), &(bits, _)| {
            (min.min(bits), max.max(bits))
        });
        let range = max.wrapping_sub(min).wrapping_add(1);
        let dense = !table.is_empty()
            && matches!(repr, Repr::I32 | Repr::I64)
            && range <= DENSE_LIMIT
            && range <= 4 * table.len() as u128 + 16;
        if dense {
            let start = u32_of(self.labels.len());
            for index in 0..range {
                let case = min + index;
                let depth = table
                    .iter()
                    .find(|&&(bits, _)| bits == case)
                    .map_or(default, |&(_, depth)| depth);
                self.labels.push(depth);
            }
            if repr == Repr::I64 {
                // Out-of-range values go to the default before the index
                // is wrapped to 32 bits.
                let index = self.temp(ValType::I64, 0);
                self.push(value);
                self.i64_const(low64(min) as i64);
                self.num(op::I64_SUB);
                self.emit(Instr::LocalTee(index));
                self.i64_const(low64(range).cast_signed());
                self.num(op::I64_GE_U);
                self.emit(Instr::BrIf(default));
                self.emit(Instr::LocalGet(index));
                self.num(op::I32_WRAP_I64);
            } else {
                self.push(value);
                if min != 0 {
                    self.i32_const(low32(min) as i32);
                    self.num(op::I32_SUB);
                }
            }
            self.emit(Instr::BrTable {
                start,
                count: low32(range),
                default,
            });
            return;
        }
        for &(bits, depth) in table {
            match repr {
                | Repr::Pair => {
                    self.push_part(value, false);
                    self.i64_const(low64(bits) as i64);
                    self.num(op::I64_EQ);
                    self.push_part(value, true);
                    self.i64_const((bits >> 64) as u64 as i64);
                    self.num(op::I64_EQ);
                    self.num(op::I32_AND);
                },
                | Repr::I64 => {
                    self.push(value);
                    self.i64_const(low64(bits) as i64);
                    self.num(op::I64_EQ);
                },
                | _ => {
                    self.push(value);
                    self.i32_const(low32(bits) as i32);
                    self.num(op::I32_EQ);
                },
            }
            self.emit(Instr::BrIf(depth));
        }
        self.emit(Instr::Br(default));
    }
}
