//! The function under construction and its SSA variables.
//!
//! Lowering appends instructions to a [`Draft`] rather than to the IR
//! builder directly, because SSA construction keeps changing the control-flow
//! edges it has already made: Braun et al.'s algorithm adds a block
//! parameter when a variable is read before all of a block's predecessors are
//! known, and later appends the matching argument to every edge into that
//! block. A draft keeps each edge's arguments in a growable list, so those
//! edits are cheap, and a parameter found redundant is merely aliased to the
//! value it equals. `emit.rs` copies the finished draft into the module.
//!
//! [`Draft::use_var`] is Cranelift's formulation of Braun's `readVariable`:
//! a lookup that would recurse into predecessors pushes [`Lookup`] calls onto
//! an explicit stack instead, and their results come back on a second stack.
//! Blocks are sealed when all their predecessors are known: a loop header
//! after its back edge, a label block at the end of the function. A read in
//! an unsealed block adds an incomplete parameter that sealing completes.
//! A variable read where no definition reaches is `poison`, C's
//! indeterminate value (C99 §6.2.4 paragraph 5, p. 32; PDF p. 44).
//!
//! Draft values and blocks reuse the IR's entity types but number the draft,
//! not the emitted function; emission renumbers them in reverse post-order.

use crate::{
    ir::{
        Block,
        Entity,
        FuncId,
        InstData,
        SigId,
        StackSlot,
        StackSlotData,
        Type,
        Value,
    },
    util::bump::{
        ArenaMap,
        ArenaSet,
        ArenaVec,
        Bump,
    },
};

/// A function body being lowered: blocks of operations, edges whose
/// arguments can still grow, and the SSA variables of its scalar locals.
pub(super) struct Draft<'s> {
    scratch:           &'s Bump,
    pub(super) blocks: ArenaVec<'s, DraftBlock<'s>>,
    pub(super) values: ArenaVec<'s, DraftValue>,
    pub(super) slots:  ArenaVec<'s, StackSlotData>,
    constants:         ArenaVec<'s, i128>,
    variables:         ArenaVec<'s, Type>,
    /// The current definition of each variable at the end of each block.
    definitions:       ArenaMap<'s, (Block, Var), Value>,
    /// One `poison` per type, materialized in the entry block on emission.
    poison:            [Option<Value>; Type::ALL.len()],
    current:           Block,
    lookups:           ArenaVec<'s, Lookup>,
    results:           ArenaVec<'s, Value>,
    visited:           ArenaSet<'s, Block>,
}

/// A scalar local whose value lives in SSA values rather than memory.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub(super) struct Var(u32);

/// One block of a draft.
pub(super) struct DraftBlock<'s> {
    /// Explicit parameters first, then those SSA construction adds.
    pub(super) params:     ArenaVec<'s, Value>,
    pub(super) insts:      ArenaVec<'s, (Op<'s>, Option<Value>)>,
    pub(super) terminator: Option<Terminator<'s>>,
    /// Each incoming edge: the block it leaves and its index there.
    pub(super) preds:      ArenaVec<'s, (Block, u32)>,
    sealed:                bool,
    /// Parameters added while the block was unsealed, still without their
    /// arguments.
    incomplete:            ArenaVec<'s, (Var, Value)>,
}

/// A draft value: its type, where it comes from, and, once SSA
/// construction found it redundant, the value it equals.
#[derive(Clone, Copy, Debug)]
pub(super) struct DraftValue {
    pub(super) ty:    Type,
    pub(super) def:   Def,
    pub(super) alias: Option<Value>,
}

/// Where a draft value is defined.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Def {
    Result,
    /// The result of an `iconst`, by its index in the draft's constants,
    /// kept so lowering can fold on it.
    Constant(u32),
    Param(Block),
    Poison,
}

/// A non-terminator operation. Operands are draft values.
#[derive(Clone, Copy, Debug)]
pub(super) enum Op<'s> {
    /// Any instruction whose operands fit in its record.
    Inst(InstData),
    Iconst(Type, i128),
    Call(FuncId, &'s [Value]),
    CallIndirect(SigId, Value, &'s [Value]),
}

/// How a block ends.
pub(super) enum Terminator<'s> {
    Jump(Edge<'s>),
    Brif(Value, [Edge<'s>; 2]),
    Switch {
        value:   Value,
        default: Edge<'s>,
        cases:   ArenaVec<'s, (i128, Edge<'s>)>,
    },
    Return(Option<Value>),
}

/// A branch to a block with arguments for its parameters.
pub(super) struct Edge<'s> {
    pub(super) target: Block,
    pub(super) args:   ArenaVec<'s, Value>,
}

/// A step of a variable lookup on the explicit stack.
#[derive(Clone, Copy)]
enum Lookup {
    /// Find the variable's value at the end of a block.
    Use(Block),
    /// The predecessors' values are on the result stack: pass them to the
    /// parameter on each edge into the block.
    FinishPredecessors(Value, Block),
}

impl Draft<'_> {
    /// Reads `var` in the current block.
    pub(super) fn use_var(&mut self, var: Var) -> Value {
        let block = self.current;
        if let Some(&value) = self.definitions.get(&(block, var)) {
            return value;
        }
        self.lookups.push(Lookup::Use(block));
        self.run_lookups(var)
    }

    /// Defines `var` in the current block.
    pub(super) fn def_var(&mut self, var: Var, value: Value) {
        _ = self.definitions.insert((self.current, var), value);
    }

    /// Declares that every predecessor of `block` is known, completing the
    /// parameters that reads added while it was open.
    pub(super) fn seal(&mut self, block: Block) {
        let data = &mut self.blocks[block.index()];
        if data.sealed {
            return;
        }
        data.sealed = true;
        let incomplete = std::mem::replace(&mut data.incomplete, ArenaVec::new_in(self.scratch));
        for (var, param) in incomplete {
            self.begin_predecessors(param, block);
            _ = self.run_lookups(var);
        }
    }

    /// Runs the lookups on the stack for `var` and returns the value the
    /// first one asked for.
    fn run_lookups(&mut self, var: Var) -> Value {
        while let Some(lookup) = self.lookups.pop() {
            match lookup {
                | Lookup::Use(block) => match self.definitions.get(&(block, var)) {
                    | Some(&value) => self.results.push(value),
                    | None => self.use_var_nonlocal(var, block),
                },
                | Lookup::FinishPredecessors(param, block) => {
                    let count = self.blocks[block.index()].preds.len();
                    let start = self.results.len() - count;
                    for index in 0..count {
                        let (pred, edge) = self.blocks[block.index()].preds[index];
                        let value = self.results[start + index];
                        self.blocks[pred.index()]
                            .terminator
                            .as_mut()
                            .expect("a predecessor ends in a branch")
                            .edge_mut(edge)
                            .args
                            .push(value);
                    }
                    self.results.truncate(start);
                    self.results.push(param);
                },
            }
        }
        self.results.pop().expect("a lookup leaves its result")
    }

    /// Finds `var` above `block`, which does not define it: along the chain
    /// of single predecessors, or in a parameter of the block where the
    /// chain ends. Every block on the chain records the result.
    fn use_var_nonlocal(&mut self, var: Var, start: Block) {
        let ty = self.variables[var.0 as usize];
        self.visited.clear();
        let mut block = start;
        let found = loop {
            let data = &self.blocks[block.index()];
            if !(data.sealed && data.preds.len() == 1 && self.visited.insert(block)) {
                break None;
            }
            block = data.preds[0].0;
            if let Some(&value) = self.definitions.get(&(block, var)) {
                self.results.push(value);
                break Some(value);
            }
        };
        let value = found.unwrap_or_else(|| {
            let data = &self.blocks[block.index()];
            if data.sealed && data.preds.is_empty() {
                let value = self.poison(ty);
                self.results.push(value);
                return value;
            }
            let sealed = data.sealed;
            let param = self.add_param(block, ty);
            if sealed {
                self.begin_predecessors(param, block);
            } else {
                self.blocks[block.index()].incomplete.push((var, param));
                self.results.push(param);
            }
            param
        });
        _ = self.definitions.insert((block, var), value);
        let mut on_path = start;
        while on_path != block {
            _ = self.definitions.insert((on_path, var), value);
            on_path = self.blocks[on_path.index()].preds[0].0;
        }
    }

    /// Schedules a lookup in each predecessor of `block`, then passing the
    /// results to `param`.
    fn begin_predecessors(&mut self, param: Value, block: Block) {
        self.lookups.push(Lookup::FinishPredecessors(param, block));
        let preds = &self.blocks[block.index()].preds;
        // The first predecessor's lookup runs first, so results arrive in
        // predecessor order.
        for &(pred, _) in preds.iter().rev() {
            self.lookups.push(Lookup::Use(pred));
        }
    }
}

// Blocks and edges

impl<'s> Draft<'s> {
    /// A new block with explicit parameters of these types.
    pub(super) fn create_block(&mut self, params: &[Type]) -> Block {
        let block = Block::new(self.blocks.len());
        self.blocks.push(DraftBlock {
            params:     ArenaVec::new_in(self.scratch),
            insts:      ArenaVec::new_in(self.scratch),
            terminator: None,
            preds:      ArenaVec::new_in(self.scratch),
            sealed:     false,
            incomplete: ArenaVec::new_in(self.scratch),
        });
        for &ty in params {
            _ = self.add_param(block, ty);
        }
        block
    }

    /// A new sealed block without predecessors, for code after a jump.
    pub(super) fn create_dead_block(&mut self) -> Block {
        let block = self.create_block(&[]);
        self.blocks[block.index()].sealed = true;
        block
    }

    pub(super) fn switch_to(&mut self, block: Block) {
        self.current = block;
    }

    pub(super) fn block_params(&self, block: Block) -> &[Value] {
        &self.blocks[block.index()].params
    }

    pub(super) fn entry() -> Block {
        Block::new(0)
    }

    fn add_param(&mut self, block: Block, ty: Type) -> Value {
        let value = self.new_value(ty, Def::Param(block));
        self.blocks[block.index()].params.push(value);
        value
    }

    /// Ends the current block with `terminator`, recording each edge as a
    /// predecessor of its target.
    pub(super) fn terminate(&mut self, terminator: Terminator<'s>) {
        let block = self.current;
        debug_assert!(
            self.blocks[block.index()].terminator.is_none(),
            "{block} is already terminated"
        );
        for (index, edge) in terminator.edges().enumerate() {
            let target = &mut self.blocks[edge.target.index()];
            debug_assert!(
                !target.sealed,
                "{} gains an edge after sealing",
                edge.target
            );
            target.preds.push((
                block,
                u32::try_from(index).expect("a terminator has few edges"),
            ));
        }
        self.blocks[block.index()].terminator = Some(terminator);
    }

    pub(super) fn edge(&self, target: Block, args: &[Value]) -> Edge<'s> {
        let mut list = ArenaVec::with_capacity_in(args.len(), self.scratch);
        list.extend_from_slice(args);
        Edge { target, args: list }
    }

    pub(super) fn jump(&mut self, target: Block, args: &[Value]) {
        let edge = self.edge(target, args);
        self.terminate(Terminator::Jump(edge));
    }

    pub(super) fn brif(&mut self, cond: Value, then: Block, otherwise: Block) {
        let edges = [self.edge(then, &[]), self.edge(otherwise, &[])];
        self.terminate(Terminator::Brif(cond, edges));
    }

    /// Ends the current block, then continues in a block nothing reaches.
    pub(super) fn terminate_and_continue_dead(&mut self, terminator: Terminator<'s>) {
        self.terminate(terminator);
        let dead = self.create_dead_block();
        self.switch_to(dead);
    }

    pub(super) fn scratch(&self) -> &'s Bump {
        self.scratch
    }
}

// Values and operations

impl<'s> Draft<'s> {
    pub(super) fn declare_var(&mut self, ty: Type) -> Var {
        let var = Var(u32::try_from(self.variables.len()).expect("too many variables"));
        self.variables.push(ty);
        var
    }

    pub(super) fn value_type(&self, value: Value) -> Type {
        self.values[value.index()].ty
    }

    /// The constant an `iconst` defines, if `value` is one.
    pub(super) fn constant(&self, value: Value) -> Option<i128> {
        match self.values[self.resolve(value).index()].def {
            | Def::Constant(index) => Some(self.constants[index as usize]),
            | _ => None,
        }
    }

    /// The value `value` stands for once redundant parameters are removed.
    pub(super) fn resolve(&self, mut value: Value) -> Value {
        while let Some(alias) = self.values[value.index()].alias {
            value = alias;
        }
        value
    }

    pub(super) fn alias(&mut self, value: Value, to: Value) {
        self.values[value.index()].alias = Some(to);
    }

    pub(super) fn poison(&mut self, ty: Type) -> Value {
        let slot = &mut self.poison[ty as usize];
        if let Some(value) = *slot {
            return value;
        }
        let value = Value::new(self.values.len());
        self.values.push(DraftValue {
            ty,
            def: Def::Poison,
            alias: None,
        });
        self.poison[ty as usize] = Some(value);
        value
    }

    /// Appends an operation to the current block; its result, if any, has
    /// type `result`.
    pub(super) fn push(&mut self, op: Op<'s>, result: Option<Type>) -> Option<Value> {
        debug_assert!(
            self.blocks[self.current.index()].terminator.is_none(),
            "{} is already terminated",
            self.current
        );
        let def = match op {
            | Op::Iconst(_, constant) => {
                let index = u32::try_from(self.constants.len()).expect("too many constants");
                self.constants.push(constant);
                Def::Constant(index)
            },
            | _ => Def::Result,
        };
        let value = result.map(|ty| self.new_value(ty, def));
        self.blocks[self.current.index()].insts.push((op, value));
        value
    }

    pub(super) fn create_slot(&mut self, size: u32, align: crate::ir::Align) -> StackSlot {
        let slot = StackSlot::new(self.slots.len());
        self.slots.push(StackSlotData { size, align });
        slot
    }

    fn new_value(&mut self, ty: Type, def: Def) -> Value {
        let value = Value::new(self.values.len());
        self.values.push(DraftValue {
            ty,
            def,
            alias: None,
        });
        value
    }
}

impl<'s> Terminator<'s> {
    /// Every edge, in the order the IR lists them: a `brif`'s targets, or a
    /// `switch`'s default and then its cases.
    pub(super) fn edges(&self) -> impl Iterator<Item = &Edge<'s>> {
        let (first, rest): (&[Edge<'s>], &[(i128, Edge<'s>)]) = match self {
            | Self::Jump(edge) => (std::slice::from_ref(edge), &[]),
            | Self::Brif(_, edges) => (edges, &[]),
            | Self::Switch { default, cases, .. } => (std::slice::from_ref(default), cases),
            | Self::Return(_) => (&[], &[]),
        };
        first.iter().chain(rest.iter().map(|(_, edge)| edge))
    }

    fn edge_mut(&mut self, index: u32) -> &mut Edge<'s> {
        let index = index as usize;
        match self {
            | Self::Jump(edge) if index == 0 => edge,
            | Self::Brif(_, edges) => &mut edges[index],
            | Self::Switch { default, .. } if index == 0 => default,
            | Self::Switch { cases, .. } => &mut cases[index - 1].1,
            | _ => unreachable!("edge {index} does not exist"),
        }
    }
}

impl<'s> Draft<'s> {
    /// An empty draft whose entry block has parameters of these types.
    pub(super) fn new(scratch: &'s Bump, params: &[Type]) -> Self {
        let mut draft = Self {
            scratch,
            blocks: ArenaVec::new_in(scratch),
            values: ArenaVec::new_in(scratch),
            slots: ArenaVec::new_in(scratch),
            constants: ArenaVec::new_in(scratch),
            variables: ArenaVec::new_in(scratch),
            definitions: ArenaMap::with_hasher_in(rustc_hash::FxBuildHasher, scratch),
            poison: [None; Type::ALL.len()],
            current: Block::new(0),
            lookups: ArenaVec::new_in(scratch),
            results: ArenaVec::new_in(scratch),
            visited: ArenaSet::with_hasher_in(rustc_hash::FxBuildHasher, scratch),
        };
        let entry = draft.create_block(params);
        draft.blocks[entry.index()].sealed = true;
        draft
    }
}
