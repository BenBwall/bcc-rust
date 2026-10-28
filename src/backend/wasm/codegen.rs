//! One IR function as a WebAssembly function body.
//!
//! [`generate`] gives every SSA value that needs one a local (two `i64`
//! locals for an `i128` or `f128`), lays out the function's frame on the
//! shadow stack, writes the prologue, and then lets the stackifier walk the
//! control-flow graph, translating each block's instructions as it reaches
//! them and each edge as an assignment of the target's parameters followed
//! by a `br` (or by the target's code). Constants and the addresses of
//! globals, functions, stack slots and labels are cheap to recompute, so
//! they get no local and are emitted where they are used.
//!
//! Values narrower than 32 bits (`i1`, `i8`, `i16`) live in `i32` locals
//! zero-extended: arithmetic that can carry into the upper bits masks its
//! result, and signed operations sign-extend their operands first.
//! Pointers are `i32`; a `ptr_add`'s 64-bit offset wraps to 32 bits, as
//! address arithmetic does on wasm32.
//!
//! The frame holds the function's stack slots, a 16-byte area for results
//! returned in memory (`i128`, `f128`, and runtime routines returning
//! them), the buffer variadic calls pass their extra arguments in, and the
//! return area of a result returned as a scalar. Global 0,
//! `__stack_pointer`, is moved down by the frame's size on entry and
//! restored on every return; dynamic allocations move it further.
//!
//! Files by role: `codegen/values.rs` (locals and operands),
//! `codegen/ops.rs` (arithmetic, comparisons and conversions),
//! `codegen/memory.rs` (loads, stores, atomics and the frame),
//! `codegen/calls.rs` (calls, returns and runtime routines),
//! `codegen/control.rs` (the stackifier's emitter: edges, branches and
//! selections).

mod calls;
mod control;
mod memory;
mod ops;
mod values;

pub(crate) use calls::{
    Routine,
    registers_scalar,
    routines_of,
    wasm_signature,
};
use values::Repr;

use super::{
    instr::{
        Instr,
        ValType,
    },
    link::Linked,
    module::Function,
    stackify::{
        self,
        Exit,
        Graph,
    },
    u32_of,
};
use crate::{
    ir::{
        BlockCall,
        Body,
        Entity,
        FuncId,
        InstData,
        Module,
        Signature,
        Type,
    },
    util::bump::{
        ArenaString,
        ArenaVec,
        Bump,
    },
};

/// Translates the body of `func` in input unit `unit`.
pub(crate) fn generate<'a>(
    link: &Linked<'a, '_>,
    unit: usize,
    func: FuncId,
    arena: &'a Bump,
    scratch: &Bump,
) -> Result<Function<'a>, &'static str> {
    let module = link.module(unit);
    let function = module.function(func);
    let body = function
        .body
        .as_ref()
        .expect("only defined functions are generated");
    let signature = module.signature(function.signature);
    let mut codegen = Codegen {
        link,
        unit,
        module,
        body,
        signature,
        arena,
        out: ArenaVec::new_in(arena),
        labels: ArenaVec::new_in(arena),
        locals: ArenaVec::new_in(arena),
        names: ArenaVec::new_in(arena),
        value_locals: ArenaVec::new_in(scratch),
        frame: memory::Frame::default(),
        frame_slots: ArenaVec::new_in(scratch),
        fp: 0,
        sp: 0,
        varargs: None,
        result_pointer: None,
        label: None,
        temps: ArenaVec::new_in(scratch),
        edge_calls: ArenaVec::new_in(scratch),
        edge_starts: ArenaVec::new_in(scratch),
        block_labels: ArenaVec::new_in(scratch),
        dispatching: false,
        error: None,
    };
    codegen.run(scratch)?;
    let params = link.function_type(unit, function.signature).params.len();
    Ok(Function {
        name:        link.symbol_name(unit, func),
        ty:          link.type_of(unit, function.signature),
        locals:      arena.alloc_slice_copy(&codegen.locals[params..]),
        local_names: arena.alloc_slice_copy(&codegen.names),
        code:        arena.alloc_slice_copy(&codegen.out),
        labels:      arena.alloc_slice_copy(&codegen.labels),
    })
}

/// The state of translating one function.
pub(super) struct Codegen<'l, 'a, 'm, 'ir, 's> {
    link:           &'l Linked<'a, 'ir>,
    unit:           usize,
    module:         &'m Module<'ir>,
    body:           &'m Body<'ir>,
    signature:      Signature<'ir>,
    arena:          &'a Bump,
    out:            ArenaVec<'a, Instr>,
    labels:         ArenaVec<'a, u32>,
    /// Every local's type, parameters first.
    locals:         ArenaVec<'a, ValType>,
    names:          ArenaVec<'a, &'a str>,
    /// Each value's first local, or `u32::MAX` for a value recomputed where
    /// it is used.
    value_locals:   ArenaVec<'s, u32>,
    frame:          memory::Frame,
    /// Each stack slot's offset from the frame pointer.
    frame_slots:    ArenaVec<'s, u32>,
    /// The frame pointer local and the local holding the stack pointer on
    /// entry, when the function moves the stack pointer.
    fp:             u32,
    sp:             u32,
    /// The parameter holding the variadic argument buffer's address.
    varargs:        Option<u32>,
    /// The parameter holding the address an `i128` or `f128` result is
    /// stored at.
    result_pointer: Option<u32>,
    /// The dispatch loop's label local.
    label:          Option<u32>,
    /// Scratch locals by type, reused across instructions.
    temps:          ArenaVec<'s, (ValType, u32)>,
    /// Each edge's target and arguments: block `b`'s are
    /// `edge_calls[edge_starts[b]..edge_starts[b + 1]]`.
    edge_calls:     ArenaVec<'s, BlockCall>,
    edge_starts:    ArenaVec<'s, u32>,
    /// Each block's label, for `indirect_br`.
    block_labels:   ArenaVec<'s, u32>,
    dispatching:    bool,
    /// The first construct the back end refused, reported after the walk.
    error:          Option<&'static str>,
}

impl Codegen<'_, '_, '_, '_, '_> {
    fn run(&mut self, scratch: &Bump) -> Result<(), &'static str> {
        self.assign_locals()?;
        self.layout_frame()?;
        self.prologue();
        let (exits, starts, targets) = self.graph(scratch)?;
        let graph = Graph {
            exits:   &exits,
            starts:  &starts,
            targets: &targets,
        };
        if self.body.has_indirect_control() {
            // An `indirect_br` selects among its listed edges like a
            // switch; the label values are numbered per function.
            self.block_labels.resize(self.body.block_count(), u32::MAX);
            for (label, block) in self.body.labels() {
                self.block_labels[block.index()] = label.as_u32();
            }
        }
        self.dispatching = stackify::stackify(&graph, self, scratch);
        self.out.push(Instr::Unreachable);
        match self.error {
            | Some(construct) => Err(construct),
            | None => Ok(()),
        }
    }

    /// The stackifier's view of the body: each block's exit and edges.
    /// A `switch`'s edges are its distinct targets with their arguments,
    /// the default first.
    #[expect(
        clippy::type_complexity,
        reason = "The three parallel tables of a graph."
    )]
    fn graph<'s>(
        &mut self,
        scratch: &'s Bump,
    ) -> Result<(ArenaVec<'s, Exit>, ArenaVec<'s, u32>, ArenaVec<'s, u32>), &'static str> {
        let count = self.body.block_count();
        let mut exits = ArenaVec::with_capacity_in(count, scratch);
        let mut starts = ArenaVec::with_capacity_in(count + 1, scratch);
        let mut targets = ArenaVec::new_in(scratch);
        for block in self.body.blocks() {
            starts.push(u32_of(targets.len()));
            self.edge_starts.push(u32_of(self.edge_calls.len()));
            let Some(terminator) = self.body.terminator(block) else {
                return Err("a block without a terminator");
            };
            let exit = match *self.body.inst(terminator) {
                | InstData::Return { .. } | InstData::Unreachable => Exit::Leave,
                | InstData::Jump { dest } => {
                    self.edge_calls.push(dest);
                    Exit::Jump
                },
                | InstData::Brif { dests, .. } => {
                    self.edge_calls.extend(dests);
                    Exit::Branch
                },
                | InstData::Switch { default, cases, .. } => {
                    let first = self.edge_calls.len();
                    self.edge_calls.push(default);
                    for (_, dest) in self.body.switch_cases(cases) {
                        if !self.edge_calls[first..]
                            .iter()
                            .any(|&known| self.same_edge(known, dest))
                        {
                            self.edge_calls.push(dest);
                        }
                    }
                    Exit::Select
                },
                | InstData::IndirectBr { dests, .. } => {
                    self.edge_calls.extend(self.body.block_list(dests));
                    Exit::Select
                },
                | _ => return Err("a block ends without a terminator"),
            };
            exits.push(exit);
            for &call in &self.edge_calls[self.edge_starts[block.index()] as usize..] {
                targets.push(self.body.block_call(call).0.as_u32());
            }
        }
        starts.push(u32_of(targets.len()));
        self.edge_starts.push(u32_of(self.edge_calls.len()));
        Ok((exits, starts, targets))
    }

    /// Whether two edges go to one block with the same arguments.
    fn same_edge(&self, a: BlockCall, b: BlockCall) -> bool {
        self.body.block_call(a) == self.body.block_call(b)
    }

    // Output helpers.

    fn emit(&mut self, instr: Instr) {
        self.out.push(instr);
    }

    fn num(&mut self, opcode: u8) {
        self.out.push(Instr::Num(opcode));
    }

    fn i32_const(&mut self, value: i32) {
        self.out.push(Instr::I32Const(value));
    }

    fn i64_const(&mut self, value: i64) {
        self.out.push(Instr::I64Const(value));
    }

    /// A new local of type `ty` named `name`.
    fn local(&mut self, ty: ValType, name: &str) -> u32 {
        let index = u32_of(self.locals.len());
        self.locals.push(ty);
        self.names.push(self.arena.alloc_str(name));
        index
    }

    /// Scratch local number `skip` of type `ty`, distinct from the others
    /// of that type an instruction asks for with another `skip`. The
    /// locals before it are created first, so asking for number 1 before
    /// number 0 does not hand the same local out twice.
    fn temp(&mut self, ty: ValType, skip: usize) -> u32 {
        loop {
            let found = self
                .temps
                .iter()
                .filter(|(temp_ty, _)| *temp_ty == ty)
                .nth(skip)
                .map(|&(_, local)| local);
            if let Some(local) = found {
                return local;
            }
            let mut name = ArenaString::new_in(self.arena);
            _ = core::fmt::Write::write_fmt(
                &mut name,
                format_args!("t{}.{}", self.temps.len(), ty.name()),
            );
            let local = self.local(ty, &name);
            self.temps.push((ty, local));
        }
    }

    /// Records a construct the back end cannot translate; translation
    /// continues so the structure stays consistent, and the function is
    /// refused at the end.
    fn refuse(&mut self, construct: &'static str) {
        if self.error.is_none() {
            self.error = Some(construct);
        }
    }

    fn repr(&mut self, ty: Type) -> Repr {
        match Repr::of(ty) {
            | Some(repr) => repr,
            | None => {
                self.refuse("the x87 80-bit floating format");
                Repr::I32
            },
        }
    }
}
