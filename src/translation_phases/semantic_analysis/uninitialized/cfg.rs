//! The control-flow graph Clang's uninitialized-values analysis runs on,
//! built as Clang's `CFGBuilder` builds it for C: backward from the end of
//! the function, creating a block whenever control can enter in the middle
//! and pruning the edges a known condition rules out
//! (`PruneTriviallyFalseEdges`). The analysis reads the graph's paths, but
//! its reports read the blocks themselves: which block ends in which
//! terminator, which label starts a block, and the order the blocks were
//! created in, which orders the variables it reports. So the builder follows
//! Clang's visitors one for one (`clang/lib/Analysis/CFG.cpp`), keeping only
//! the elements the analysis transfers: references to tracked variables,
//! assignments to them, their declarations and calls that return twice.
//!
//! An expression the graph would not hold exactly as Clang's does leaves the
//! function unsupported, and the analysis then reports nothing in it: a
//! missed warning is better than one Clang does not give.
//! C99: §6.8, pp. 131-139; PDF pp. 143-151; §5.1.2.3p2, p. 13; PDF p. 25.

use rustc_hash::FxBuildHasher;

use super::super::{
    Analyzer,
    BindingKind,
    Duration,
    ExpressionType as E,
    Integer,
    Linkage,
    types::{
        Scalar,
        TagKind,
        TypeId,
        TypeKind,
    },
};
use crate::{
    translation_phases::{
        parsing::{
            Builtin,
            OffsetMember,
            SyntaxOperand,
            declaration_syntax::{
                Declaration,
                Declarator,
                DirectDeclarator,
                Initializer,
                InitializerType,
                TypeQualifiers,
            },
            syntax::{
                BinaryOperator as B,
                BlockItem,
                ConstantExpressionSlot,
                Expression,
                ExpressionSlot,
                ForInitializer,
                Statement,
                StatementType as S,
                UnaryOperator as U,
            },
        },
        preprocessing::KeywordTokenType,
    },
    util::{
        bump::{
            ArenaMap,
            ArenaVec,
            Bump,
        },
        string_cache::StringCacheId,
    },
};

/// A block's index, in the order the builder created it: Clang's block ID.
pub(super) type BlockId = u32;

/// How a reference to a tracked variable reads it (Clang's
/// `ClassifyRefs::Class`), ordered as Clang orders the classes: where two
/// rules classify one reference, the greater wins.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub(super) enum Class {
    /// The reference writes or escapes: `&x`, an `asm` output, the base of
    /// `s.m`. No rule classified it.
    Init,
    /// The value is read.
    Use,
    /// `int x = x;`, which leaves `x` uninitialized on purpose.
    SelfInit,
    /// `&x` passed where a function takes a pointer to `const`.
    ConstPtrUse,
    /// The value is discarded: `(void)x`, the left operand of a comma, the
    /// target of `=`.
    Ignore,
}

/// What a block element does to the tracked variables.
#[derive(Clone, Copy)]
pub(super) enum Element<'tu> {
    /// A reference to tracked variable `var`.
    Reference {
        var:   u32,
        class: Class,
        user:  &'tu Expression<'tu>,
    },
    /// `x = ...` stores into tracked variable `x`.
    Assign(u32),
    /// The declaration of a tracked variable, with an initializer that
    /// initializes it or without one.
    Declare { var: u32, initialized: bool },
    /// A call to a function that returns twice: every variable may now be
    /// initialized.
    ReturnsTwice,
    /// A call through a declaration marked `analyzer_noreturn`: every
    /// variable's value is unknown, as if the path went no further.
    Unknown,
}

/// The statement or expression whose condition ends a block, as far as the
/// reports name it.
#[derive(Clone, Copy)]
pub(super) enum Terminator<'tu> {
    If(&'tu Statement<'tu>),
    While(&'tu Statement<'tu>),
    Do(&'tu Statement<'tu>),
    For(&'tu Statement<'tu>),
    Switch,
    /// `&&` or `||`.
    Logical(&'tu Expression<'tu>),
    /// `?:` with all three operands.
    Conditional(&'tu Expression<'tu>),
    /// A jump, or a terminator the reports do not name: `?:` without its
    /// middle operand, `goto`, `break`, `continue`, a computed `goto`.
    Other,
}

/// The label that starts a block: a switch reports the label of the
/// successor its condition reaches.
#[derive(Clone, Copy)]
pub(super) enum Label<'tu> {
    Case(&'tu Expression<'tu>),
    Default(&'tu Statement<'tu>),
    Named,
}

/// Where a tracked variable is declared, for the notes that name it.
#[derive(Clone, Copy)]
pub(super) struct Declared<'tu> {
    pub(super) declaration: &'tu Declaration<'tu>,
    pub(super) declarator:  Declarator<'tu>,
    pub(super) initializer: Option<&'tu Initializer<'tu>>,
}

#[derive(Clone, Copy)]
pub(super) struct Block<'tu> {
    pub(super) terminator: Option<Terminator<'tu>>,
    pub(super) label:      Option<Label<'tu>>,
}

/// The finished graph: blocks in creation order with their successors (a
/// pruned edge is `None`), their predecessors along edges that are not
/// pruned, and their elements in execution order.
pub(super) struct Cfg<'b, 'tu> {
    pub(super) blocks:     &'b [Block<'tu>],
    pub(super) succs:      &'b [Option<BlockId>],
    pub(super) succ_start: &'b [u32],
    pub(super) preds:      &'b [BlockId],
    pub(super) pred_start: &'b [u32],
    pub(super) elements:   &'b [Element<'tu>],
    pub(super) elem_start: &'b [u32],
    pub(super) entry:      BlockId,
    /// The bindings of the tracked variables, by variable index.
    pub(super) vars:       &'b [usize],
    /// Their declarations, which a use in unevaluated code may lack.
    pub(super) declared:   &'b [Option<Declared<'tu>>],
}

impl<'tu> Cfg<'_, 'tu> {
    pub(super) fn successors(&self, block: BlockId) -> &[Option<BlockId>] {
        let b = block as usize;
        &self.succs[self.succ_start[b] as usize..self.succ_start[b + 1] as usize]
    }

    pub(super) fn predecessors(&self, block: BlockId) -> &[BlockId] {
        let b = block as usize;
        &self.preds[self.pred_start[b] as usize..self.pred_start[b + 1] as usize]
    }

    pub(super) fn elements(&self, block: BlockId) -> &[Element<'tu>] {
        let b = block as usize;
        &self.elements[self.elem_start[b] as usize..self.elem_start[b + 1] as usize]
    }
}

/// How the context of an expression classifies a reference it reaches,
/// through parentheses and the operators Clang's `ClassifyRefs::classify`
/// and `stripCasts` look through.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Context {
    /// No rule: a reference here initializes.
    Plain,
    /// The value is read.
    Use,
    /// The value is discarded.
    Ignore,
    /// The value is discarded if the expression is a reference through
    /// parentheses and casts that change nothing (Clang's `findVar`), as
    /// after such a conversion of a discarded value; otherwise it is read.
    IgnoreVariable,
    /// The initializer of the variable's own declaration.
    SelfInit,
    /// The operand of `&` in an argument of pointer-to-`const` type.
    ConstPtr { trivial: bool },
    /// An argument of pointer-to-`const` type, above its `&`.
    ConstPtrArgument { trivial: bool },
}

/// The builder's state: Clang's `CFGBuilder` fields that C reaches.
pub(super) struct Builder<'b, 'tu> {
    arena:                  &'b Bump,
    blocks:                 ArenaVec<'b, Block<'tu>>,
    /// Each edge: its source, its target if any, and whether it is
    /// reachable.
    edges:                  ArenaVec<'b, (BlockId, Option<BlockId>, bool)>,
    /// Elements as appended, in reverse execution order within a block.
    elements:               ArenaVec<'b, (BlockId, Element<'tu>)>,
    block:                  Option<BlockId>,
    succ:                   Option<BlockId>,
    exit:                   BlockId,
    break_target:           Option<BlockId>,
    continue_target:        Option<BlockId>,
    switch_block:           Option<BlockId>,
    default_case:           Option<BlockId>,
    /// The constant value of the innermost switch's condition.
    switch_value:           Option<Integer>,
    /// A case matched the innermost switch's constant condition.
    switch_covered:         bool,
    labels:                 ArenaMap<'b, StringCacheId, BlockId>,
    backpatch:              ArenaVec<'b, (BlockId, StringCacheId)>,
    address_taken:          ArenaVec<'b, StringCacheId>,
    indirect:               Option<BlockId>,
    /// A `break` or `continue` with nowhere to go: Clang builds no graph.
    bad:                    bool,
    /// Something the builder does not model as Clang does.
    pub(super) unsupported: bool,
    vars:                   ArenaMap<'b, usize, u32>,
    var_bindings:           ArenaVec<'b, usize>,
    declared:               ArenaMap<'b, usize, Declared<'tu>>,
    /// Conditions already decided, as `tryEvaluateBool` caches them.
    pub(super) known:       ArenaMap<'b, usize, Option<bool>>,
}

impl<'b, 'tu> Builder<'b, 'tu> {
    pub(super) fn new(arena: &'b Bump) -> Self {
        let mut builder = Self {
            arena,
            blocks: ArenaVec::new_in(arena),
            edges: ArenaVec::new_in(arena),
            elements: ArenaVec::new_in(arena),
            block: None,
            succ: None,
            exit: 0,
            break_target: None,
            continue_target: None,
            switch_block: None,
            default_case: None,
            switch_value: None,
            switch_covered: false,
            labels: ArenaMap::with_hasher_in(FxBuildHasher, arena),
            backpatch: ArenaVec::new_in(arena),
            address_taken: ArenaVec::new_in(arena),
            indirect: None,
            bad: false,
            unsupported: false,
            vars: ArenaMap::with_hasher_in(FxBuildHasher, arena),
            var_bindings: ArenaVec::new_in(arena),
            declared: ArenaMap::with_hasher_in(FxBuildHasher, arena),
            known: ArenaMap::with_hasher_in(FxBuildHasher, arena),
        };
        // The exit block comes first, as Clang's does (`buildCFG`).
        builder.exit = builder.create_block(false);
        builder.succ = Some(builder.exit);
        builder
    }

    fn create_block(&mut self, add_successor: bool) -> BlockId {
        let id = BlockId::try_from(self.blocks.len()).unwrap_or(BlockId::MAX);
        self.blocks.push(Block {
            terminator: None,
            label:      None,
        });
        if add_successor && let Some(succ) = self.succ {
            self.add_successor(id, Some(succ), true);
        }
        id
    }

    /// A block a call that does not return ends: its only successor is the
    /// exit (`createNoReturnBlock`).
    fn create_noreturn_block(&mut self) -> BlockId {
        let id = self.create_block(false);
        self.add_successor(id, Some(self.exit), true);
        id
    }

    fn auto_create_block(&mut self) -> BlockId {
        match self.block {
            | Some(block) => block,
            | None => {
                let block = self.create_block(true);
                self.block = Some(block);
                block
            },
        }
    }

    fn add_successor(&mut self, from: BlockId, to: Option<BlockId>, reachable: bool) {
        self.edges.push((from, to, reachable));
    }

    fn terminate(&mut self, block: BlockId, terminator: Terminator<'tu>) {
        self.blocks[block as usize].terminator = Some(terminator);
    }

    fn label(&mut self, block: BlockId, label: Label<'tu>) {
        self.blocks[block as usize].label = Some(label);
    }

    fn append(&mut self, element: Element<'tu>) {
        let block = self.auto_create_block();
        self.elements.push((block, element));
    }

    /// The variable index of tracked binding `binding`.
    fn var(&mut self, binding: usize) -> u32 {
        if let Some(&var) = self.vars.get(&binding) {
            return var;
        }
        let var = u32::try_from(self.var_bindings.len()).unwrap_or(u32::MAX);
        self.var_bindings.push(binding);
        _ = self.vars.insert(binding, var);
        var
    }

    /// Lays the graph out in arrays, once the body is built: the edges
    /// `goto` waited for, the computed `goto` block's successors and the
    /// entry. `None` when Clang builds no graph.
    pub(super) fn finish(mut self, body_entry: Option<BlockId>) -> Option<Cfg<'b, 'tu>> {
        if self.bad || self.unsupported {
            return None;
        }
        if let Some(entry) = body_entry {
            self.succ = Some(entry);
        }
        let backpatch = std::mem::replace(&mut self.backpatch, ArenaVec::new_in(self.arena));
        for &(block, name) in &backpatch {
            if let Some(&target) = self.labels.get(&name) {
                self.add_successor(block, Some(target), true);
            }
        }
        if let Some(indirect) = self.indirect {
            let taken = std::mem::replace(&mut self.address_taken, ArenaVec::new_in(self.arena));
            let mut seen: ArenaMap<'_, StringCacheId, ()> =
                ArenaMap::with_hasher_in(FxBuildHasher, self.arena);
            for &name in &taken {
                if seen.insert(name, ()).is_none()
                    && let Some(&target) = self.labels.get(&name)
                {
                    self.add_successor(indirect, Some(target), true);
                }
            }
        }
        let entry = self.create_block(true);
        let count = self.blocks.len();
        let arena = self.arena;
        // Successor lists keep the order the edges were added in, and a block
        // runs its elements in the reverse of the order they were appended.
        let succ_start = arena.alloc_slice_fill_iter(std::iter::repeat_n(0_u32, count + 1));
        let pred_start = arena.alloc_slice_fill_iter(std::iter::repeat_n(0_u32, count + 1));
        let elem_start = arena.alloc_slice_fill_iter(std::iter::repeat_n(0_u32, count + 1));
        for &(from, to, reachable) in &self.edges {
            succ_start[from as usize + 1] += 1;
            if reachable && let Some(to) = to {
                pred_start[to as usize + 1] += 1;
            }
        }
        for &(block, _) in &self.elements {
            elem_start[block as usize + 1] += 1;
        }
        for index in 0..count {
            succ_start[index + 1] += succ_start[index];
            pred_start[index + 1] += pred_start[index];
            elem_start[index + 1] += elem_start[index];
        }
        let succs =
            arena.alloc_slice_fill_iter(std::iter::repeat_n(None, succ_start[count] as usize));
        let preds = arena.alloc_slice_fill_iter(std::iter::repeat_n(0, pred_start[count] as usize));
        let elements = arena.alloc_slice_fill_iter(std::iter::repeat_n(
            Element::ReturnsTwice,
            elem_start[count] as usize,
        ));
        let succ_fill = arena.alloc_slice_copy(&succ_start[..count]);
        let pred_fill = arena.alloc_slice_copy(&pred_start[..count]);
        for &(from, to, reachable) in &self.edges {
            let slot = &mut succ_fill[from as usize];
            succs[*slot as usize] = to.filter(|_| reachable);
            *slot += 1;
            if reachable && let Some(to) = to {
                let slot = &mut pred_fill[to as usize];
                preds[*slot as usize] = from;
                *slot += 1;
            }
        }
        let elem_fill = arena.alloc_slice_copy(&elem_start[1..]);
        for &(block, element) in &self.elements {
            let slot = &mut elem_fill[block as usize];
            *slot -= 1;
            elements[*slot as usize] = element;
        }
        let declared = arena.alloc_slice_fill_iter(
            self.var_bindings
                .iter()
                .map(|binding| self.declared.get(binding).copied()),
        );
        Some(Cfg {
            declared,
            blocks: self.blocks.leak(),
            succs,
            succ_start,
            preds,
            pred_start,
            elements,
            elem_start,
            entry,
            vars: self.var_bindings.leak(),
        })
    }
}

impl<'tu> Analyzer<'_, 'tu, '_> {
    /// Builds the graph of the current function's `body`, as `buildCFG`
    /// does; `None` when Clang builds none or the builder cannot follow it.
    pub(super) fn build_uninitialized_cfg<'b>(
        &mut self,
        mut cfg: Builder<'b, 'tu>,
        body: &'tu Statement<'tu>,
    ) -> Option<Cfg<'b, 'tu>> {
        let entry = self.cfg_statement(&mut cfg, body);
        cfg.finish(entry)
    }

    /// Clang's `CFGBuilder::Visit` of a statement, returning the block that
    /// starts it, if any.
    /// C99: §6.8, pp. 131-139; PDF pp. 143-151.
    fn cfg_statement(
        &mut self,
        cfg: &mut Builder<'_, 'tu>,
        statement: &'tu Statement<'tu>,
    ) -> Option<BlockId> {
        if cfg.bad || cfg.unsupported {
            return None;
        }
        match statement.kind {
            | S::Compound { items } => {
                let mut last = cfg.block;
                for item in items.iter().rev() {
                    let block = match *item {
                        | BlockItem::Statement(statement) => self.cfg_statement(cfg, statement),
                        | BlockItem::Declaration(declaration) =>
                            self.cfg_declaration(cfg, declaration),
                        | BlockItem::FunctionDefinition(_) => {
                            cfg.unsupported = true;
                            None
                        },
                    };
                    if block.is_some() {
                        last = block;
                    }
                    if cfg.bad || cfg.unsupported {
                        return None;
                    }
                }
                last
            },
            | S::Declaration(declaration) => self.cfg_declaration(cfg, declaration),
            | S::Expression(slot) => self.cfg_slot(cfg, slot),
            | S::Null => cfg.block,
            | S::Attributed(attributed) => self.cfg_statement(cfg, attributed.statement),
            | S::If {
                condition_expression,
                then_statement,
                else_statement,
            } => self.cfg_if(
                cfg,
                statement,
                condition_expression,
                then_statement,
                else_statement,
            ),
            | S::While {
                condition_expression,
                body_statement,
            } => self.cfg_while(cfg, statement, condition_expression, body_statement),
            | S::DoWhile {
                condition_expression,
                body_statement,
            } => self.cfg_do(cfg, statement, condition_expression, body_statement),
            | S::For(_) => self.cfg_for(cfg, statement),
            | S::Switch {
                condition_expression,
                body_statement,
            } => self.cfg_switch(cfg, statement, condition_expression, body_statement),
            | S::Case(bound, child) => self.cfg_case(cfg, statement, bound, child),
            | S::CaseRange(range) => self.cfg_case(cfg, statement, range.lower, range.statement),
            | S::Default(child) => {
                _ = self.cfg_statement(cfg, child);
                let block = cfg.block.unwrap_or_else(|| cfg.create_block(true));
                cfg.default_case = Some(block);
                cfg.label(block, Label::Default(statement));
                cfg.block = None;
                cfg.succ = Some(block);
                Some(block)
            },
            | S::Label(name, child) => {
                _ = self.cfg_statement(cfg, child);
                let block = cfg.block.unwrap_or_else(|| cfg.create_block(true));
                _ = cfg.labels.insert(name.name, block);
                cfg.label(block, Label::Named);
                cfg.block = None;
                cfg.succ = Some(block);
                Some(block)
            },
            | S::Goto(name) => {
                let block = cfg.create_block(false);
                cfg.block = Some(block);
                cfg.terminate(block, Terminator::Other);
                match cfg.labels.get(&name.name) {
                    | Some(&target) => cfg.add_successor(block, Some(target), true),
                    | None => cfg.backpatch.push((block, name.name)),
                }
                Some(block)
            },
            | S::ComputedGoto(slot) => {
                let indirect = match cfg.indirect {
                    | Some(block) => block,
                    | None => {
                        let block = cfg.create_block(false);
                        cfg.indirect = Some(block);
                        block
                    },
                };
                let block = cfg.create_block(false);
                cfg.block = Some(block);
                cfg.terminate(block, Terminator::Other);
                cfg.add_successor(block, Some(indirect), true);
                self.cfg_slot(cfg, slot)
            },
            | S::Break | S::Continue => {
                let block = cfg.create_block(false);
                cfg.block = Some(block);
                cfg.terminate(block, Terminator::Other);
                let target = if matches!(statement.kind, S::Break) {
                    cfg.break_target
                } else {
                    cfg.continue_target
                };
                match target {
                    | Some(target) => cfg.add_successor(block, Some(target), true),
                    | None => cfg.bad = true,
                }
                Some(block)
            },
            | S::Return(slot) => {
                let block = cfg.create_block(false);
                cfg.block = Some(block);
                cfg.add_successor(block, Some(cfg.exit), true);
                match slot {
                    | Some(slot) => self.cfg_slot(cfg, slot),
                    | None => Some(block),
                }
            },
            | S::Asm(asm) => {
                if asm.qualifiers.goto {
                    cfg.unsupported = true;
                    return None;
                }
                _ = cfg.auto_create_block();
                let mut last = cfg.block;
                // Children run outputs first, then inputs; the builder visits
                // them backward.
                for operand in asm.operands.iter().rev() {
                    let context = if operand.output {
                        Context::Plain
                    } else {
                        match self.input_constraint_reads(operand.constraint) {
                            | Some(true) => Context::Use,
                            | Some(false) => Context::Plain,
                            | None => {
                                cfg.unsupported = true;
                                return None;
                            },
                        }
                    };
                    if let Some(block) = self.cfg_expression(cfg, operand.expression, context) {
                        last = Some(block);
                    }
                }
                last
            },
            | S::MsAsm(_)
            | S::Seh(_)
            | S::SehLeave
            | S::NamedBreak(_)
            | S::NamedContinue(_)
            | S::LocalLabels(_) => {
                cfg.unsupported = true;
                None
            },
        }
    }

    fn cfg_slot(
        &mut self,
        cfg: &mut Builder<'_, 'tu>,
        slot: ExpressionSlot<'tu>,
    ) -> Option<BlockId> {
        match slot {
            | ExpressionSlot::Parsed(e) => self.cfg_expression(cfg, e, Context::Use),
            | ExpressionSlot::Selection(_) | ExpressionSlot::Missing(_) => {
                cfg.unsupported = true;
                None
            },
        }
    }

    /// A condition's expression, which a selection header or a missing
    /// expression leaves the builder unable to follow.
    fn condition(
        cfg: &mut Builder<'_, 'tu>,
        slot: ExpressionSlot<'tu>,
    ) -> Option<&'tu Expression<'tu>> {
        match slot {
            | ExpressionSlot::Parsed(e) => Some(e),
            | _ => {
                cfg.unsupported = true;
                None
            },
        }
    }

    /// Clang's `VisitIfStmt`.
    /// C99: §6.8.4.1, pp. 133-134; PDF pp. 145-146.
    fn cfg_if(
        &mut self,
        cfg: &mut Builder<'_, 'tu>,
        statement: &'tu Statement<'tu>,
        condition: ExpressionSlot<'tu>,
        then_statement: &'tu Statement<'tu>,
        else_statement: Option<&'tu Statement<'tu>>,
    ) -> Option<BlockId> {
        let condition = Self::condition(cfg, condition)?;
        if cfg.block.is_some() {
            cfg.succ = cfg.block;
        }
        let mut else_block = cfg.succ;
        if let Some(else_statement) = else_statement {
            let saved = cfg.succ;
            cfg.block = None;
            else_block = self.cfg_statement(cfg, else_statement).or(saved);
            cfg.succ = saved;
        }
        let then_block = {
            let saved = cfg.succ;
            cfg.block = None;
            let block = match self.cfg_statement(cfg, then_statement) {
                | Some(block) => block,
                | None => {
                    let block = cfg.create_block(false);
                    cfg.add_successor(block, saved, true);
                    block
                },
            };
            cfg.succ = saved;
            block
        };
        if cfg.bad || cfg.unsupported {
            return None;
        }
        let stripped = self.cfg_strip(cfg, condition)?;
        if Self::logical(stripped).is_some() {
            return self
                .cfg_logical(
                    cfg,
                    stripped,
                    Some(Terminator::If(statement)),
                    Some(then_block),
                    else_block,
                )
                .0;
        }
        let block = cfg.create_block(false);
        cfg.block = Some(block);
        cfg.terminate(block, Terminator::If(statement));
        let known = self.cfg_known(cfg, condition);
        cfg.add_successor(block, Some(then_block), known != Some(false));
        cfg.add_successor(block, else_block, known != Some(true));
        self.cfg_expression(cfg, condition, Context::Use)
    }

    /// Clang's `VisitWhileStmt`.
    /// C99: §6.8.5.1, p. 136; PDF p. 148.
    fn cfg_while(
        &mut self,
        cfg: &mut Builder<'_, 'tu>,
        statement: &'tu Statement<'tu>,
        condition: ExpressionSlot<'tu>,
        body: &'tu Statement<'tu>,
    ) -> Option<BlockId> {
        let condition = Self::condition(cfg, condition)?;
        let loop_successor = match cfg.block {
            | Some(block) => {
                cfg.block = None;
                Some(block)
            },
            | None => cfg.succ,
        };
        let (saved_block, saved_succ) = (cfg.block, cfg.succ);
        let (saved_continue, saved_break) = (cfg.continue_target, cfg.break_target);
        let transition = cfg.create_block(false);
        cfg.succ = Some(transition);
        cfg.continue_target = Some(transition);
        cfg.break_target = loop_successor;
        let body_block = self.cfg_statement(cfg, body).unwrap_or(transition);
        (cfg.block, cfg.succ) = (saved_block, saved_succ);
        (cfg.continue_target, cfg.break_target) = (saved_continue, saved_break);
        if cfg.bad || cfg.unsupported {
            return None;
        }
        let stripped = self.cfg_strip(cfg, condition)?;
        let entry = if Self::logical(stripped).is_some() {
            self.cfg_logical(
                cfg,
                stripped,
                Some(Terminator::While(statement)),
                Some(body_block),
                loop_successor,
            )
            .0
        } else {
            let exit = cfg.create_block(false);
            cfg.terminate(exit, Terminator::While(statement));
            cfg.block = Some(exit);
            let entry = self.cfg_expression(cfg, condition, Context::Use);
            cfg.block = entry;
            let known = self.cfg_known(cfg, condition);
            cfg.add_successor(exit, (known != Some(false)).then_some(body_block), true);
            cfg.add_successor(exit, loop_successor.filter(|_| known != Some(true)), true);
            entry
        };
        if let Some(entry) = entry {
            cfg.add_successor(transition, Some(entry), true);
        } else {
            cfg.unsupported = true;
        }
        cfg.block = None;
        cfg.succ = entry;
        entry
    }

    /// Clang's `VisitDoStmt`.
    /// C99: §6.8.5.2, p. 136; PDF p. 148.
    fn cfg_do(
        &mut self,
        cfg: &mut Builder<'_, 'tu>,
        statement: &'tu Statement<'tu>,
        condition: ExpressionSlot<'tu>,
        body: &'tu Statement<'tu>,
    ) -> Option<BlockId> {
        let condition = Self::condition(cfg, condition)?;
        let loop_successor = cfg.block.or(cfg.succ);
        let exit = cfg.create_block(false);
        cfg.terminate(exit, Terminator::Do(statement));
        cfg.block = Some(exit);
        let entry_condition = self
            .cfg_expression(cfg, condition, Context::Use)
            .unwrap_or(exit);
        cfg.succ = Some(entry_condition);
        let known = self.cfg_known(cfg, condition);
        let (saved_block, saved_succ) = (cfg.block, cfg.succ);
        let (saved_continue, saved_break) = (cfg.continue_target, cfg.break_target);
        cfg.continue_target = Some(entry_condition);
        cfg.break_target = loop_successor;
        cfg.block = None;
        let body_block = self.cfg_statement(cfg, body).unwrap_or(entry_condition);
        cfg.block = None;
        cfg.succ = Some(body_block);
        let loop_back = cfg.create_block(true);
        if known == Some(false) {
            cfg.add_successor(exit, None, true);
        } else {
            cfg.add_successor(exit, Some(loop_back), true);
        }
        (cfg.block, cfg.succ) = (saved_block, saved_succ);
        (cfg.continue_target, cfg.break_target) = (saved_continue, saved_break);
        cfg.add_successor(exit, loop_successor.filter(|_| known != Some(true)), true);
        cfg.block = None;
        cfg.succ = Some(body_block);
        Some(body_block)
    }

    /// Clang's `VisitForStmt`.
    /// C99: §6.8.5.3, p. 136; PDF p. 148.
    fn cfg_for(
        &mut self,
        cfg: &mut Builder<'_, 'tu>,
        statement: &'tu Statement<'tu>,
    ) -> Option<BlockId> {
        let S::For(for_loop) = statement.kind else {
            return None;
        };
        let condition = match for_loop.condition_expression {
            | Some(slot) => Some(Self::condition(cfg, slot)?),
            | None => None,
        };
        let loop_successor = cfg.block.or(cfg.succ);
        let saved_break = cfg.break_target;
        cfg.break_target = loop_successor;
        let (saved_block, saved_succ, saved_continue) = (cfg.block, cfg.succ, cfg.continue_target);
        let transition = cfg.create_block(false);
        cfg.block = Some(transition);
        cfg.succ = Some(transition);
        if let Some(slot) = for_loop.iteration_expression {
            cfg.succ = self.cfg_slot(cfg, slot);
        }
        if cfg.block.is_some() {
            cfg.block = None;
        }
        cfg.continue_target = cfg.succ;
        let body_block = self
            .cfg_statement(cfg, for_loop.body_statement)
            .or(cfg.continue_target);
        (cfg.block, cfg.succ, cfg.continue_target) = (saved_block, saved_succ, saved_continue);
        if cfg.bad || cfg.unsupported {
            return None;
        }
        let stripped = match condition {
            | Some(condition) => Some(self.cfg_strip(cfg, condition)?),
            | None => None,
        };
        let entry = if let Some(stripped) = stripped.filter(|&e| Self::logical(e).is_some()) {
            self.cfg_logical(
                cfg,
                stripped,
                Some(Terminator::For(statement)),
                body_block,
                loop_successor,
            )
            .0
        } else {
            let exit = cfg.create_block(false);
            cfg.terminate(exit, Terminator::For(statement));
            let mut entry = Some(exit);
            let mut known = Some(true);
            if let Some(condition) = condition {
                cfg.block = Some(exit);
                entry = self.cfg_expression(cfg, condition, Context::Use);
                known = self.cfg_known(cfg, condition);
            }
            cfg.add_successor(exit, body_block.filter(|_| known != Some(false)), true);
            cfg.add_successor(exit, loop_successor.filter(|_| known != Some(true)), true);
            entry
        };
        cfg.add_successor(transition, entry, true);
        cfg.succ = entry;
        let result = match for_loop.initializer {
            | Some(initializer) => {
                let block = cfg.create_block(true);
                cfg.block = Some(block);
                match initializer {
                    | ForInitializer::Declaration(declaration) =>
                        self.cfg_declaration(cfg, declaration),
                    | ForInitializer::Expression(slot) => self.cfg_slot(cfg, slot),
                }
            },
            | None => {
                cfg.block = None;
                cfg.succ = entry;
                entry
            },
        };
        cfg.break_target = saved_break;
        result
    }

    /// Clang's `VisitSwitchStmt`.
    /// C99: §6.8.4.2, pp. 134-135; PDF pp. 146-147.
    fn cfg_switch(
        &mut self,
        cfg: &mut Builder<'_, 'tu>,
        statement: &'tu Statement<'tu>,
        condition: ExpressionSlot<'tu>,
        body: &'tu Statement<'tu>,
    ) -> Option<BlockId> {
        let condition_expression = Self::condition(cfg, condition)?;
        let switch_successor = cfg.block.or(cfg.succ);
        let saved = (
            cfg.switch_block,
            cfg.default_case,
            cfg.break_target,
            cfg.switch_covered,
            cfg.switch_value,
        );
        cfg.default_case = switch_successor;
        let switch_block = cfg.create_block(false);
        cfg.switch_block = Some(switch_block);
        cfg.succ = switch_successor;
        cfg.break_target = switch_successor;
        cfg.block = None;
        cfg.switch_covered = false;
        let facts = self.switch_facts(statement, condition);
        cfg.switch_value = facts.value;
        _ = self.cfg_statement(cfg, body);
        if cfg.bad || cfg.unsupported {
            return None;
        }
        // `switchExclusivelyCovered`, or every enumerator has a case.
        let always = cfg.switch_covered || (facts.covered && facts.value.is_none());
        let default_case = cfg.default_case;
        cfg.add_successor(switch_block, default_case, !always);
        cfg.terminate(switch_block, Terminator::Switch);
        cfg.block = Some(switch_block);
        let last = self.cfg_expression(cfg, condition_expression, Context::Use);
        (
            cfg.switch_block,
            cfg.default_case,
            cfg.break_target,
            cfg.switch_covered,
            cfg.switch_value,
        ) = saved;
        last
    }

    /// Clang's `shouldAddCase`: whether the innermost switch's constant
    /// condition can reach the case, which ends the search once one does.
    fn cfg_should_add_case(
        &self,
        cfg: &mut Builder<'_, 'tu>,
        bound: ConstantExpressionSlot<'tu>,
        upper: ConstantExpressionSlot<'tu>,
    ) -> bool {
        let Some(value) = cfg.switch_value else {
            return true;
        };
        if cfg.switch_covered {
            return false;
        }
        let matched = self
            .case_value_range(bound, upper)
            .is_some_and(|(low, high)| Self::in_case_range(value, low, high));
        if matched {
            cfg.switch_covered = true;
        }
        matched
    }

    /// Clang's `VisitCaseStmt`, which gives each case directly inside
    /// another its own block.
    /// C99: §6.8.4.2 paragraph 4, p. 134; PDF p. 146.
    fn cfg_case(
        &mut self,
        cfg: &mut Builder<'_, 'tu>,
        statement: &'tu Statement<'tu>,
        bound: ConstantExpressionSlot<'tu>,
        child: &'tu Statement<'tu>,
    ) -> Option<BlockId> {
        let Some(switch_block) = cfg.switch_block else {
            cfg.bad = true;
            return None;
        };
        let bounds = |statement: &'tu Statement<'tu>| match statement.kind {
            | S::Case(bound, child) => Some((bound, bound, child)),
            | S::CaseRange(range) => Some((range.lower, range.upper, range.statement)),
            | _ => None,
        };
        let (mut low, mut high, mut sub) = bounds(statement).unwrap_or((bound, bound, child));
        let mut top = None;
        let mut last = None;
        while let Some((next_low, next_high, next_sub)) = bounds(sub) {
            let current = cfg.create_block(false);
            cfg.label(current, Label::Case(Self::case_bound(low)?));
            if let (Some(_), Some(last)) = (top, last) {
                cfg.add_successor(last, Some(current), true);
            } else {
                top = Some(current);
            }
            let add = self.cfg_should_add_case(cfg, low, high);
            cfg.add_successor(switch_block, add.then_some(current), true);
            last = Some(current);
            (low, high, sub) = (next_low, next_high, next_sub);
        }
        _ = self.cfg_statement(cfg, sub);
        let case_block = cfg.block.unwrap_or_else(|| cfg.create_block(true));
        cfg.label(case_block, Label::Case(Self::case_bound(low)?));
        if cfg.bad || cfg.unsupported {
            return None;
        }
        let add = self.cfg_should_add_case(cfg, low, high);
        cfg.add_successor(switch_block, Some(case_block), add);
        cfg.block = None;
        if let (Some(top), Some(last)) = (top, last) {
            cfg.add_successor(last, Some(case_block), true);
            cfg.succ = Some(top);
        } else {
            cfg.succ = Some(case_block);
        }
        cfg.succ
    }

    fn case_bound(bound: ConstantExpressionSlot<'tu>) -> Option<&'tu Expression<'tu>> {
        match bound {
            | ConstantExpressionSlot::Parsed(bound) => Some(bound.expression()),
            | ConstantExpressionSlot::Missing(_) => None,
        }
    }

    /// `tryEvaluateBool` of a condition, cached as Clang caches it.
    fn cfg_known(&mut self, cfg: &mut Builder<'_, 'tu>, e: &'tu Expression<'tu>) -> Option<bool> {
        let mut known = std::mem::replace(
            &mut cfg.known,
            ArenaMap::with_hasher_in(FxBuildHasher, cfg.arena),
        );
        let result = self.known_condition_in(&mut known, e);
        cfg.known = known;
        result
    }

    /// The `&&` or `||` operator `e` is, if any.
    fn logical(e: &'tu Expression<'tu>) -> Option<B> {
        match e.kind {
            | E::Binary {
                operator: operator @ (B::LogicalAnd | B::LogicalOr),
                ..
            } => Some(operator),
            | _ => None,
        }
    }

    /// Clang's `Expr::IgnoreParens`: through parentheses, `__extension__`,
    /// the association a `_Generic` selects and the operand a
    /// `__builtin_choose_expr` chooses. A selection the analysis does not
    /// know leaves the builder unable to follow.
    fn cfg_strip(
        &self,
        cfg: &mut Builder<'_, 'tu>,
        mut e: &'tu Expression<'tu>,
    ) -> Option<&'tu Expression<'tu>> {
        loop {
            e = match e.kind {
                | E::Parenthesized { expression }
                | E::Unary {
                    operator: U::Extension,
                    operand_expression: expression,
                } => expression,
                | E::Generic(generic) => {
                    let selected = self
                        .reachability
                        .selected_generic
                        .get(&std::ptr::from_ref(generic).addr())
                        .copied();
                    let Some(association) = generic.associations.iter().find(|association| {
                        Some(std::ptr::from_ref(association.expression).addr()) == selected
                    }) else {
                        cfg.unsupported = true;
                        return None;
                    };
                    association.expression
                },
                | E::Builtin(builtin) if builtin.keyword == KeywordTokenType::BuiltinChooseExpr => {
                    let [
                        SyntaxOperand::Expression(condition),
                        SyntaxOperand::Expression(yes),
                        SyntaxOperand::Expression(no),
                    ] = *builtin.operands.as_slice()
                    else {
                        cfg.unsupported = true;
                        return None;
                    };
                    match Self::constant_truth(self.expression_info(condition)) {
                        | Some(true) => yes,
                        | Some(false) => no,
                        | None => {
                            cfg.unsupported = true;
                            return None;
                        },
                    }
                },
                | _ => return Some(e),
            };
        }
    }

    /// Clang's `VisitLogicalOperator` for `e`, an `&&` or `||` whose operands
    /// branch to `true_block` and `false_block`, `term` ending the block of
    /// its right operand; with no `term`, they are one block, where its value
    /// is used. Returns the blocks that start and end it.
    /// C99: §6.5.13p4, p. 89; PDF p. 101; §6.5.14p4, p. 89; PDF p. 101.
    fn cfg_logical(
        &mut self,
        cfg: &mut Builder<'_, 'tu>,
        e: &'tu Expression<'tu>,
        term: Option<Terminator<'tu>>,
        true_block: Option<BlockId>,
        false_block: Option<BlockId>,
    ) -> (Option<BlockId>, Option<BlockId>) {
        let E::Binary {
            operator,
            left_expression,
            right_expression,
        } = e.kind
        else {
            cfg.unsupported = true;
            return (None, None);
        };
        let Some(right) = self.cfg_strip(cfg, right_expression) else {
            return (None, None);
        };
        let (right_block, exit_block) = if Self::logical(right).is_some() {
            self.cfg_logical(cfg, right, term, true_block, false_block)
        } else {
            let block = cfg.create_block(false);
            let mut known = self.cfg_known(cfg, right);
            if known.is_none() {
                known = self.cfg_known(cfg, e);
            }
            match term {
                | None => cfg.add_successor(block, true_block, true),
                | Some(term) => {
                    cfg.terminate(block, term);
                    cfg.add_successor(block, true_block, known != Some(false));
                    cfg.add_successor(block, false_block, known != Some(true));
                },
            }
            cfg.block = Some(block);
            (self.cfg_expression(cfg, right, Context::Use), Some(block))
        };
        if cfg.bad || cfg.unsupported {
            return (None, None);
        }
        let Some(left) = self.cfg_strip(cfg, left_expression) else {
            return (None, None);
        };
        if Self::logical(left).is_some() {
            let (true_block, false_block) = if operator == B::LogicalOr {
                (true_block, right_block)
            } else {
                (right_block, false_block)
            };
            let (entry, _) = self.cfg_logical(
                cfg,
                left,
                Some(Terminator::Logical(e)),
                true_block,
                false_block,
            );
            return (entry, exit_block);
        }
        let left_block = cfg.create_block(false);
        cfg.terminate(left_block, Terminator::Logical(e));
        cfg.block = Some(left_block);
        let entry = self.cfg_expression(cfg, left, Context::Use);
        let known = self.cfg_known(cfg, left);
        if operator == B::LogicalOr {
            cfg.add_successor(left_block, true_block, known != Some(false));
            cfg.add_successor(left_block, right_block, known != Some(true));
        } else {
            cfg.add_successor(left_block, right_block, known != Some(false));
            cfg.add_successor(left_block, false_block, known != Some(true));
        }
        (entry, exit_block)
    }

    /// Clang's `VisitConditionalOperator`, for `?:` with or without its
    /// middle operand.
    /// C99: §6.5.15p4, p. 90; PDF p. 102.
    fn cfg_conditional(
        &mut self,
        cfg: &mut Builder<'_, 'tu>,
        e: &'tu Expression<'tu>,
        context: Context,
    ) -> Option<BlockId> {
        let (c, omitted) = match e.kind {
            | E::Conditional(c) => (c, false),
            | E::OmittedConditional(c) => (c, true),
            | _ => return None,
        };
        let confluence = cfg.block.unwrap_or_else(|| cfg.create_block(true));
        cfg.succ = Some(confluence);
        cfg.block = None;
        let then_block = if omitted {
            Some(confluence)
        } else {
            let arm = self.arm_context(e, c.then_expression, context);
            let block = self.cfg_expression(cfg, c.then_expression, arm);
            cfg.block = None;
            block
        };
        cfg.succ = Some(confluence);
        let arm = self.arm_context(e, c.else_expression, context);
        let else_block = self.cfg_expression(cfg, c.else_expression, arm);
        if cfg.bad || cfg.unsupported {
            return None;
        }
        let condition = self.cfg_strip(cfg, c.condition_expression)?;
        if !omitted && Self::logical(condition).is_some() {
            return self
                .cfg_logical(
                    cfg,
                    condition,
                    Some(Terminator::Conditional(e)),
                    then_block,
                    else_block,
                )
                .0;
        }
        let block = cfg.create_block(false);
        cfg.block = Some(block);
        let known = self.cfg_known(cfg, c.condition_expression);
        cfg.add_successor(block, then_block, known != Some(false));
        cfg.add_successor(block, else_block, known != Some(true));
        cfg.terminate(
            block,
            if omitted {
                Terminator::Other
            } else {
                Terminator::Conditional(e)
            },
        );
        self.cfg_expression(cfg, c.condition_expression, Context::Use)
    }

    /// The context in which an arm `arm` of the conditional `conditional`
    /// in `context` is evaluated. Clang's `ClassifyRefs::classify` passes a
    /// discarded value on to an arm that its conversion to the result's
    /// type leaves as it is; through a conversion that `stripCasts` steps
    /// through it finds only a variable, and through any other it finds
    /// nothing, so the arm's value is read.
    /// C99: §6.5.15p5, p. 90; PDF p. 102; §6.3.1.8p1, pp. 44-45; PDF pp. 56-57.
    fn arm_context(
        &self,
        conditional: &'tu Expression<'tu>,
        arm: &'tu Expression<'tu>,
        context: Context,
    ) -> Context {
        if context != Context::Ignore {
            return Context::Use;
        }
        let to = self.expression_info(conditional).ty;
        let from = self.expression_info(arm).ty;
        if from.unqualified() == to.unqualified() {
            Context::Ignore
        } else if self.noop_conversion(from, to) {
            Context::IgnoreVariable
        } else {
            Context::Use
        }
    }

    /// Clang's `VisitDeclStmt`: each declarator on its own, the last first.
    /// A variable's declaration runs after its bounds and its initializer.
    /// C99: §6.7, pp. 97-98; PDF pp. 109-110; §6.8p3, p. 131; PDF p. 143.
    fn cfg_declaration(
        &mut self,
        cfg: &mut Builder<'_, 'tu>,
        declaration: &'tu Declaration<'tu>,
    ) -> Option<BlockId> {
        let mut last = None;
        for init in declaration.init_declarators.iter().rev() {
            let Some(binding) = self.correspondence.binding_of(init.declarator) else {
                // Rejected declarators leave the function with an error.
                cfg.unsupported = true;
                return None;
            };
            let data = self.bindings[binding];
            let block = match data.kind {
                | BindingKind::Typedef =>
                    if self.types.variably_modified(data.ty) {
                        _ = cfg.auto_create_block();
                        let mut block = cfg.block;
                        self.cfg_bounds(cfg, Some(init.declarator), data.ty, &mut block);
                        block
                    } else {
                        cfg.block
                    },
                | BindingKind::Object => {
                    let tracked = self.tracked_binding(binding);
                    let self_init = tracked
                        && init.initializer.is_some_and(|initializer| {
                            self.self_initializer(initializer, binding).is_some()
                        });
                    if tracked {
                        let var = cfg.var(binding);
                        _ = cfg.declared.insert(
                            binding,
                            Declared {
                                declaration,
                                declarator: init.declarator,
                                initializer: init.initializer,
                            },
                        );
                        cfg.append(Element::Declare {
                            var,
                            initialized: init.initializer.is_some() && !self_init,
                        });
                    } else {
                        _ = cfg.auto_create_block();
                    }
                    let mut block = cfg.block;
                    if let Some(initializer) = init.initializer {
                        let context = if self_init {
                            Context::SelfInit
                        } else {
                            Context::Use
                        };
                        if let Some(entry) = self.cfg_initializer(cfg, initializer, context) {
                            block = Some(entry);
                        }
                    }
                    self.cfg_bounds(cfg, Some(init.declarator), data.ty, &mut block);
                    block
                },
                | _ => cfg.block,
            };
            if block.is_some() {
                last = block;
            }
            if cfg.bad || cfg.unsupported {
                return None;
            }
        }
        last.or(cfg.block)
    }

    /// The variable length array bounds Clang's `FindVA` finds in the type
    /// `declarator` derives: those of the array derivations nearest the
    /// name, before any pointer or function derivation, and none of a typedef
    /// name's own type. Each bound runs before the declaration, the innermost
    /// first.
    /// C99: §6.7.5.2 paragraph 5, p. 117; PDF p. 129.
    fn cfg_bounds(
        &mut self,
        cfg: &mut Builder<'_, 'tu>,
        declarator: Option<Declarator<'tu>>,
        ty: TypeId,
        block: &mut Option<BlockId>,
    ) {
        let Some(declarator) = declarator else {
            return;
        };
        // Parentheses in a declarator make Clang's type a `ParenType`, which
        // `FindVA` does not see through; the builder does not follow them.
        let parenthesized = declarator
            .kind
            .iter()
            .any(|direct| matches!(direct, DirectDeclarator::Parenthesized(_)));
        let mut bounds: ArenaVec<'_, &'tu Expression<'tu>> = ArenaVec::new_in(cfg.arena);
        let mut ty = ty;
        for direct in declarator.kind {
            match *direct {
                | DirectDeclarator::Array {
                    assignment_expression,
                    ..
                } => {
                    let TypeKind::Array(element, bound) = self.types.nodes[ty.index] else {
                        cfg.unsupported = true;
                        return;
                    };
                    if matches!(bound, super::super::ArrayBound::Variable(_)) {
                        if parenthesized {
                            cfg.unsupported = true;
                            return;
                        }
                        bounds.extend(assignment_expression);
                    }
                    ty = element;
                },
                | DirectDeclarator::Function { .. }
                | DirectDeclarator::KAndRStyleFunction { .. } => break,
                | DirectDeclarator::Parenthesized(inner) =>
                    if self.declarator_variable_bound(inner.declarator) {
                        cfg.unsupported = true;
                        return;
                    },
                | _ => {},
            }
        }
        for &bound in &bounds {
            if let Some(entry) = self.cfg_expression(cfg, bound, Context::Use) {
                *block = Some(entry);
            }
        }
    }

    /// Whether a declarator writes an array bound that is not a constant.
    fn declarator_variable_bound(&self, declarator: Declarator<'tu>) -> bool {
        declarator.kind.iter().any(|direct| match *direct {
            | DirectDeclarator::Array {
                assignment_expression: Some(bound),
                ..
            } => self.expression_info(bound).integer.is_none(),
            | DirectDeclarator::Parenthesized(inner) =>
                self.declarator_variable_bound(inner.declarator),
            | _ => false,
        })
    }

    /// An initializer: an expression, or a list whose elements run in order.
    /// A list whose order Clang's semantic form changes, with designators or
    /// elements it drops, is followed only when it reads no tracked
    /// variable.
    /// C99: §6.7.8, pp. 125-130; PDF pp. 137-142.
    fn cfg_initializer(
        &mut self,
        cfg: &mut Builder<'_, 'tu>,
        initializer: &'tu Initializer<'tu>,
        context: Context,
    ) -> Option<BlockId> {
        match initializer.kind {
            | InitializerType::AssignmentExpression(e) => self.cfg_expression(cfg, e, context),
            | InitializerType::InitializerList(list) => {
                _ = cfg.auto_create_block();
                let mut entry = cfg.block;
                if designated(initializer) && self.reads_tracked(initializer) {
                    cfg.unsupported = true;
                    return None;
                }
                for element in list.elements.iter().rev() {
                    if let Some(block) =
                        self.cfg_initializer(cfg, element.initializer, Context::Use)
                    {
                        entry = Some(block);
                    }
                }
                entry
            },
        }
    }

    /// Whether `initializer` names a variable the analysis could track.
    fn reads_tracked(&self, initializer: &'tu Initializer<'tu>) -> bool {
        match initializer.kind {
            | InitializerType::AssignmentExpression(e) => self.names_tracked(e),
            | InitializerType::InitializerList(list) => list
                .elements
                .iter()
                .any(|element| self.reads_tracked(element.initializer)),
        }
    }

    /// Whether expression `e` names a variable the analysis could track,
    /// evaluated or not.
    fn names_tracked(&self, e: &'tu Expression<'tu>) -> bool {
        let mut found = false;
        let mut stack: ArenaVec<'_, &'tu Expression<'tu>> = ArenaVec::new_in(self.scratch);
        stack.push(e);
        while let Some(e) = stack.pop() {
            match e.kind {
                | E::Identifier(_) => {
                    if self
                        .expression_info(e)
                        .binding
                        .is_some_and(|binding| self.tracked_binding(binding))
                    {
                        found = true;
                        break;
                    }
                },
                | E::Parenthesized { expression }
                | E::Unary {
                    operand_expression: expression,
                    ..
                }
                | E::Cast {
                    operand_expression: expression,
                    ..
                }
                | E::DirectMember {
                    base_expression: expression,
                    ..
                }
                | E::IndirectMember {
                    base_expression: expression,
                    ..
                }
                | E::SizeofExpr(expression)
                | E::AlignofExpr(expression) => stack.push(expression),
                | E::Binary {
                    left_expression,
                    right_expression,
                    ..
                } => {
                    stack.push(left_expression);
                    stack.push(right_expression);
                },
                | E::Conditional(c) | E::OmittedConditional(c) => {
                    stack.push(c.condition_expression);
                    stack.push(c.then_expression);
                    stack.push(c.else_expression);
                },
                | E::Call {
                    function_expression,
                    arguments,
                } => {
                    stack.push(function_expression);
                    stack.extend(arguments.iter().copied());
                },
                | E::Generic(generic) => {
                    stack.extend(generic.associations.iter().map(|a| a.expression));
                    if let SyntaxOperand::Expression(e) = generic.controlling {
                        stack.push(e);
                    }
                },
                | E::Builtin(builtin) =>
                    for operand in builtin.operands {
                        if let SyntaxOperand::Expression(e) = *operand {
                            stack.push(e);
                        }
                    },
                // A statement expression or a compound literal may hold
                // anything; take them as naming one.
                | E::StatementExpression(_) | E::CompoundLiteral { .. } => {
                    found = true;
                    break;
                },
                | _ => {},
            }
        }
        found
    }

    /// Clang's `CFGBuilder::Visit` of an expression, every node of which
    /// the builder adds as Clang adds the nodes the analysis asks for. The
    /// reference `e` reaches through the operators that pass `context` on is
    /// classified by it.
    /// C99: §6.5, pp. 67-94; PDF pp. 79-106; §6.3.2.1p2, p. 46; PDF p. 58.
    fn cfg_expression(
        &mut self,
        cfg: &mut Builder<'_, 'tu>,
        e: &'tu Expression<'tu>,
        context: Context,
    ) -> Option<BlockId> {
        if cfg.bad || cfg.unsupported {
            return None;
        }
        let e = self.cfg_strip(cfg, e)?;
        match e.kind {
            | E::Identifier(_) => {
                let binding = self.expression_info(e).binding;
                match binding.filter(|&binding| self.tracked_binding(binding)) {
                    | Some(binding) => {
                        let var = cfg.var(binding);
                        let class = match context {
                            | Context::Plain => Class::Init,
                            | Context::Use | Context::ConstPtrArgument { .. } => Class::Use,
                            | Context::Ignore
                            | Context::IgnoreVariable
                            | Context::ConstPtr { trivial: true } => Class::Ignore,
                            | Context::SelfInit => Class::SelfInit,
                            | Context::ConstPtr { trivial: false } => Class::ConstPtrUse,
                        };
                        cfg.append(Element::Reference {
                            var,
                            class,
                            user: e,
                        });
                    },
                    | None => _ = cfg.auto_create_block(),
                }
                cfg.block
            },
            | E::Binary {
                operator: B::LogicalAnd | B::LogicalOr,
                ..
            } => {
                let confluence = cfg.block.unwrap_or_else(|| cfg.create_block(true));
                self.cfg_logical(cfg, e, None, Some(confluence), Some(confluence))
                    .0
            },
            | E::Binary {
                operator: B::Comma,
                left_expression,
                right_expression,
            } => {
                _ = cfg.auto_create_block();
                let right = if context == Context::Ignore {
                    Context::Ignore
                } else {
                    Context::Use
                };
                _ = self.cfg_expression(cfg, right_expression, right);
                self.cfg_expression(cfg, left_expression, Context::Ignore)
            },
            | E::Binary {
                operator: B::Assignment,
                left_expression,
                right_expression,
            } => {
                match self.find_tracked(left_expression) {
                    | Some(binding) => {
                        let var = cfg.var(binding);
                        cfg.append(Element::Assign(var));
                    },
                    | None => _ = cfg.auto_create_block(),
                }
                _ = self.cfg_expression(cfg, left_expression, Context::Ignore);
                self.cfg_expression(cfg, right_expression, Context::Use)
            },
            | E::Binary {
                operator:
                    B::MultiplicationAssignment
                    | B::DivisionAssignment
                    | B::ModuloAssignment
                    | B::AdditionAssignment
                    | B::SubtractionAssignment
                    | B::LeftShiftAssignment
                    | B::RightShiftAssignment
                    | B::BitwiseAndAssignment
                    | B::BitwiseXorAssignment
                    | B::BitwiseOrAssignment,
                left_expression,
                right_expression,
            } => {
                // `CompoundAssignOperator` is not Clang's `BinaryOperator`
                // class, so its operands run in order (`VisitStmt`).
                _ = cfg.auto_create_block();
                let right = self.cfg_expression(cfg, right_expression, Context::Use);
                let left = self.cfg_expression(cfg, left_expression, Context::Use);
                left.or(right)
            },
            | E::Binary {
                operator,
                left_expression,
                right_expression,
            } => {
                _ = cfg.auto_create_block();
                // A vector's element is a subscript of the vector itself,
                // which is not converted to its value.
                let operand = |analyzer: &Self, operand: &'tu Expression<'tu>| {
                    if operator == B::Subscript
                        && matches!(
                            analyzer.types.nodes[analyzer
                                .types
                                .non_atomic(analyzer.expression_info(operand).ty)
                                .index],
                            TypeKind::Vector { .. }
                        )
                    {
                        Context::Plain
                    } else {
                        Context::Use
                    }
                };
                let (left_context, right_context) = (
                    operand(self, left_expression),
                    operand(self, right_expression),
                );
                let right = self.cfg_expression(cfg, right_expression, right_context);
                let left = self.cfg_expression(cfg, left_expression, left_context);
                left.or(right)
            },
            | E::Conditional(_) | E::OmittedConditional(_) => self.cfg_conditional(cfg, e, context),
            | E::Unary {
                operator,
                operand_expression,
            } => {
                _ = cfg.auto_create_block();
                let operand = match (operator, context) {
                    | (U::AddressOf, Context::ConstPtrArgument { trivial }) =>
                        Context::ConstPtr { trivial },
                    | (U::AddressOf | U::Real | U::Imag, _) => Context::Plain,
                    | _ => Context::Use,
                };
                self.cfg_expression(cfg, operand_expression, operand)
            },
            | E::Cast {
                operand_expression, ..
            } => {
                _ = cfg.auto_create_block();
                let target = self.expression_info(e).ty;
                let operand = if matches!(
                    self.types.nodes[self.types.non_atomic(target).index],
                    TypeKind::Scalar(Scalar::Void)
                ) {
                    Context::Ignore
                } else if self.noop_cast(e, operand_expression) {
                    match context {
                        | Context::Ignore | Context::IgnoreVariable => Context::IgnoreVariable,
                        | Context::SelfInit | Context::ConstPtrArgument { .. } => context,
                        | _ => Context::Use,
                    }
                } else {
                    Context::Use
                };
                self.cfg_expression(cfg, operand_expression, operand)
            },
            | E::DirectMember {
                base_expression, ..
            } => {
                _ = cfg.auto_create_block();
                self.cfg_expression(cfg, base_expression, Context::Plain)
            },
            | E::IndirectMember {
                base_expression, ..
            } => {
                _ = cfg.auto_create_block();
                self.cfg_expression(cfg, base_expression, Context::Use)
            },
            | E::Call {
                function_expression,
                arguments,
            } => self.cfg_call(cfg, function_expression, arguments.as_slice()),
            | E::StatementExpression(statement) => {
                _ = cfg.auto_create_block();
                self.cfg_statement(cfg, statement)
            },
            | E::CompoundLiteral { initializer, .. } => {
                _ = cfg.auto_create_block();
                self.cfg_initializer(cfg, initializer, Context::Use)
            },
            | E::LabelAddress(name) => {
                cfg.address_taken.push(name.name);
                _ = cfg.auto_create_block();
                cfg.block
            },
            | E::SizeofType(name) => {
                _ = cfg.auto_create_block();
                let mut block = cfg.block;
                let key = super::super::type_name_identity(name);
                match self.resolved_type_names.get(&key).copied() {
                    | Some(ty) => self.cfg_bounds(cfg, name.declarator, ty, &mut block),
                    | None => cfg.unsupported = true,
                }
                block
            },
            | E::Builtin(builtin) => self.cfg_builtin(cfg, builtin),
            | E::Generic(_) | E::Error => {
                cfg.unsupported = true;
                None
            },
            | E::SizeofExpr(_)
            | E::AlignofExpr(_)
            | E::AlignofType(_)
            | E::Countof(_)
            | E::Constant(_)
            | E::StringLiteral(_)
            | E::Boolean(_)
            | E::Nullptr
            | E::Parenthesized { .. } => {
                _ = cfg.auto_create_block();
                cfg.block
            },
        }
    }

    /// Clang's `VisitCallExpr`: a call that does not return ends its block;
    /// `__builtin_object_size` and an assumption with side effects evaluate
    /// no arguments. An argument of pointer-to-`const` type that takes a
    /// variable's address passes it for reading (`ClassifyRefs::
    /// VisitCallExpr`), unless the function's body is empty.
    /// C99: §6.5.2.2, pp. 71-72; PDF pp. 83-84.
    fn cfg_call(
        &mut self,
        cfg: &mut Builder<'_, 'tu>,
        callee: &'tu Expression<'tu>,
        arguments: &'tu [&'tu Expression<'tu>],
    ) -> Option<BlockId> {
        let object_size = self.intrinsic_named(
            callee,
            &["__builtin_object_size", "__builtin_dynamic_object_size"],
        );
        if object_size {
            _ = cfg.auto_create_block();
            return self.cfg_expression(cfg, callee, Context::Use);
        }
        if self.intrinsic_named(callee, &["__builtin_assume", "__assume"]) {
            // Its argument is evaluated only without side effects.
            if arguments
                .iter()
                .any(|&argument| self.has_side_effects(argument))
            {
                _ = cfg.auto_create_block();
                return self.cfg_expression(cfg, callee, Context::Use);
            }
        }
        let mut known = std::mem::replace(
            &mut cfg.known,
            ArenaMap::with_hasher_in(FxBuildHasher, cfg.arena),
        );
        let analyzer_noreturn = self.callee_analyzer_noreturn(callee);
        let noreturn = self.cfg_noreturn_call(&mut known, callee, arguments)
            || analyzer_noreturn == Some(BindingKind::Function);
        cfg.known = known;
        if noreturn {
            if cfg.block.is_some() {
                cfg.succ = cfg.block;
            }
            cfg.block = Some(cfg.create_noreturn_block());
        } else {
            _ = cfg.auto_create_block();
        }
        if self.callee_returns_twice(callee) {
            cfg.append(Element::ReturnsTwice);
        } else if analyzer_noreturn.is_some() {
            cfg.append(Element::Unknown);
        }
        let trivial = self.trivial_callee(callee);
        let parameters = self.parameter_types(callee);
        let mut last = cfg.block;
        for (index, &argument) in arguments.iter().enumerate().rev() {
            let ty = parameters
                .and_then(|parameters| parameters.get(index).copied())
                .unwrap_or_else(|| self.expression_info(argument).ty);
            let context = if self.pointer_to_const(ty) {
                Context::ConstPtrArgument { trivial }
            } else {
                Context::Use
            };
            if let Some(block) = self.cfg_expression(cfg, argument, context) {
                last = Some(block);
            }
        }
        if let Some(block) = self.cfg_expression(cfg, callee, Context::Use) {
            last = Some(block);
        }
        last
    }

    /// A GNU builtin with its own syntax: the `va_list` a `va_start`,
    /// `va_end`, `va_copy` or `va_arg` names is passed by reference, an
    /// `offsetof` evaluates its indices, and Clang reads no operand of
    /// `__builtin_convertvector` or `__builtin_bit_cast` as a use.
    /// GNU extension: GCC manual, "Other Builtins".
    fn cfg_builtin(
        &mut self,
        cfg: &mut Builder<'_, 'tu>,
        builtin: &'tu Builtin<'tu>,
    ) -> Option<BlockId> {
        _ = cfg.auto_create_block();
        let mut last = cfg.block;
        let operands: ArenaVec<'_, &'tu Expression<'tu>> = {
            let mut operands = ArenaVec::new_in(cfg.arena);
            operands.extend(
                builtin
                    .operands
                    .iter()
                    .filter_map(|operand| match *operand {
                        | SyntaxOperand::Expression(e) => Some(e),
                        | SyntaxOperand::Type(_) => None,
                    }),
            );
            operands
        };
        match builtin.keyword {
            | KeywordTokenType::BuiltinVaStart
            | KeywordTokenType::BuiltinVaEnd
            | KeywordTokenType::BuiltinVaCopy
            | KeywordTokenType::BuiltinVaArg
            | KeywordTokenType::BuiltinConvertVector
            | KeywordTokenType::BuiltinBitCast => {
                for (index, &operand) in operands.iter().enumerate().rev() {
                    let context = match builtin.keyword {
                        | KeywordTokenType::BuiltinVaStart if index > 0 => Context::Use,
                        | _ => Context::Plain,
                    };
                    if let Some(block) = self.cfg_expression(cfg, operand, context) {
                        last = Some(block);
                    }
                }
            },
            | KeywordTokenType::BuiltinOffsetof =>
                for member in builtin.members.iter().rev() {
                    if let OffsetMember::Index(index) = *member
                        && let Some(block) = self.cfg_expression(cfg, index, Context::Use)
                    {
                        last = Some(block);
                    }
                },
            | KeywordTokenType::BuiltinTypesCompatible => {},
            | _ => {
                cfg.unsupported = true;
                return None;
            },
        }
        last
    }

    /// Clang's `findVar` of an assignment's target: the tracked variable it
    /// names, through parentheses and casts that change nothing.
    fn find_tracked(&self, e: &'tu Expression<'tu>) -> Option<usize> {
        let e = self.strip_noop(e);
        if !matches!(e.kind, E::Identifier(_)) {
            return None;
        }
        self.expression_info(e)
            .binding
            .filter(|&binding| self.tracked_binding(binding))
    }

    /// Clang's `stripCasts`: `IgnoreParenNoopCasts`, through parentheses,
    /// `__extension__`, selections and casts between types of one width.
    fn strip_noop(&self, mut e: &'tu Expression<'tu>) -> &'tu Expression<'tu> {
        loop {
            e = match e.kind {
                | E::Cast {
                    operand_expression, ..
                } if self.noop_cast(e, operand_expression) => operand_expression,
                | _ => match self.parens_step(e) {
                    | Some(inner) => inner,
                    | None => return e,
                },
            };
        }
    }

    /// One step of Clang's `IgnoreParens`: the operand of parentheses or
    /// `__extension__`, a generic selection's selected association, or the
    /// operand a `__builtin_choose_expr` chooses.
    /// C11: §6.5.1.1p3, p. 79; PDF p. 97.
    fn parens_step(&self, e: &'tu Expression<'tu>) -> Option<&'tu Expression<'tu>> {
        match e.kind {
            | E::Parenthesized { expression }
            | E::Unary {
                operator: U::Extension,
                operand_expression: expression,
            } => Some(expression),
            | E::Generic(generic) => {
                let selected = self
                    .reachability
                    .selected_generic
                    .get(&std::ptr::from_ref(generic).addr())
                    .copied()?;
                generic
                    .associations
                    .iter()
                    .find(|association| {
                        std::ptr::from_ref(association.expression).addr() == selected
                    })
                    .map(|association| association.expression)
            },
            | E::Builtin(builtin) if builtin.keyword == KeywordTokenType::BuiltinChooseExpr => {
                let [
                    SyntaxOperand::Expression(condition),
                    SyntaxOperand::Expression(yes),
                    SyntaxOperand::Expression(no),
                ] = *builtin.operands.as_slice()
                else {
                    return None;
                };
                Self::constant_truth(self.expression_info(condition))
                    .map(|truth| if truth { yes } else { no })
            },
            | _ => None,
        }
    }

    /// The binding a call's function operand names, as Clang's
    /// `Expr::getReferencedDeclOfCallee` finds the declaration whose
    /// attributes and body the analysis consults: through parentheses and
    /// the implicit conversions, then through `*`, `&` and unary `+`, so
    /// that `(*f)()` and `(&f)()` call `f`. An explicit cast hides it,
    /// even one that changes nothing.
    /// C99: §6.5.2.2p1, p. 71; PDF p. 83; §6.5.3.2p4, p. 79; PDF p. 91.
    fn callee_binding(&self, callee: &'tu Expression<'tu>) -> Option<usize> {
        let mut e = callee;
        loop {
            e = match e.kind {
                | E::Unary {
                    operator: U::Indirection | U::AddressOf | U::Plus,
                    operand_expression,
                } => operand_expression,
                | _ => match self.parens_step(e) {
                    | Some(inner) => inner,
                    | None => break,
                },
            };
        }
        match e.kind {
            | E::Identifier(_) => self.expression_info(e).binding,
            | _ => None,
        }
    }

    /// Whether cast `cast` of `operand` is one Clang's
    /// `IgnoreParenNoopCasts` steps through: to the same type, or between
    /// integer and pointer types of the same width.
    fn noop_cast(&self, cast: &'tu Expression<'tu>, operand: &'tu Expression<'tu>) -> bool {
        self.noop_conversion(
            self.expression_info(operand).ty,
            self.expression_info(cast).ty,
        )
    }

    /// Whether a conversion from type `from` to type `to` is one Clang's
    /// `IgnoreParenNoopCasts` steps through.
    fn noop_conversion(&self, from: TypeId, to: TypeId) -> bool {
        if to.unqualified() == from.unqualified() {
            return true;
        }
        let integral_or_pointer = |ty: TypeId| match self.types.nodes[ty.index] {
            | TypeKind::Pointer(_) | TypeKind::BitInt { .. } => true,
            | TypeKind::Scalar(scalar) => scalar_is_integral(scalar),
            | TypeKind::Tag(tag) => self.types.tags[tag].kind == TagKind::Enum,
            | _ => false,
        };
        // Arrays and functions decay first; Clang casts their address.
        if !integral_or_pointer(to) || !integral_or_pointer(from) {
            return false;
        }
        let size = |ty: TypeId| self.types.layout(ty).map(|layout| layout.size);
        size(to).is_some() && size(to) == size(from)
    }

    /// Clang's `getSelfInitExpr`: the reference to the variable itself that
    /// a non-record initializer is, through what `stripCasts` steps through.
    pub(super) fn self_initializer(
        &self,
        initializer: &'tu Initializer<'tu>,
        binding: usize,
    ) -> Option<&'tu Expression<'tu>> {
        let InitializerType::AssignmentExpression(e) = initializer.kind else {
            return None;
        };
        if matches!(
            self.types.nodes[self.bindings[binding].ty.index],
            TypeKind::Tag(tag) if self.types.tags[tag].kind != TagKind::Enum
        ) {
            return None;
        }
        let e = self.strip_noop(e);
        (matches!(e.kind, E::Identifier(_)) && self.expression_info(e).binding == Some(binding))
            .then_some(e)
    }

    /// Clang's `isTrackedVar`: a local variable of automatic storage whose
    /// type is a scalar, a vector, or a record with a member that holds
    /// data.
    /// C99: §6.2.4p4, p. 32; PDF p. 44; §6.2.5p21, p. 36; PDF p. 48.
    pub(super) fn tracked_binding(&self, binding: usize) -> bool {
        let data = self.bindings[binding];
        data.kind == BindingKind::Object
            && data.duration == Duration::Automatic
            && data.linkage == Linkage::None
            && self.tracked_type(data.ty)
    }

    fn tracked_type(&self, ty: TypeId) -> bool {
        if ty.qualifiers.contains(TypeQualifiers::ATOMIC) {
            return false;
        }
        match self.types.nodes[ty.index] {
            | TypeKind::Scalar(Scalar::Void) => false,
            | TypeKind::Scalar(_)
            | TypeKind::Pointer(_)
            | TypeKind::Nullptr
            | TypeKind::BitInt { .. }
            | TypeKind::Vector { .. } => true,
            | TypeKind::Tag(tag) =>
                self.types.tags[tag].kind == TagKind::Enum || self.record_holds_data(tag, 0),
            | _ => false,
        }
    }

    /// Clang's `recordIsNotEmpty`: a member that is not an unnamed bit-field
    /// and is not a record without one.
    fn record_holds_data(&self, tag: usize, depth: u32) -> bool {
        if depth > 64 {
            return true;
        }
        self.types.tags[tag].members.get().iter().any(|member| {
            if member.width.is_some() && (member.name.is_none() || member.width == Some(0)) {
                return false;
            }
            match self.types.nodes[member.ty.index] {
                | TypeKind::Tag(inner) if self.types.tags[inner].kind != TagKind::Enum =>
                    self.record_holds_data(inner, depth + 1),
                | _ => true,
            }
        })
    }

    /// Clang's `isPointerToConst`.
    fn pointer_to_const(&self, ty: TypeId) -> bool {
        match self.types.nodes[self.types.non_atomic(ty).index] {
            | TypeKind::Pointer(target) => target.qualifiers.contains(TypeQualifiers::CONST),
            | _ => false,
        }
    }

    /// The parameter types of a prototype the call converts its arguments
    /// to: the called function's, or for a builtin with no C prototype, none.
    fn parameter_types(&self, callee: &'tu Expression<'tu>) -> Option<&'tu [TypeId]> {
        if let Some(descriptor) = self.intrinsic_descriptor(callee)
            && descriptor
                .signature
                .parameters
                .iter()
                .any(|parameter| matches!(parameter, crate::builtins::SignatureType::Opaque))
        {
            return None;
        }
        let mut ty = self.types.non_atomic(self.expression_info(callee).ty);
        if let TypeKind::Pointer(target) = self.types.nodes[ty.index] {
            ty = target;
        }
        match self.types.nodes[ty.index] {
            | TypeKind::Function {
                parameters,
                prototype: true,
                ..
            } => Some(parameters),
            | _ => None,
        }
    }

    /// The builtin a call designates directly, if any.
    fn intrinsic_descriptor(
        &self,
        callee: &'tu Expression<'tu>,
    ) -> Option<&'static crate::builtins::BuiltinDescriptor> {
        let binding = self.callee_binding(callee)?;
        match self.bindings[binding].origin {
            | super::super::BindingOrigin::Intrinsic(id) => Some(id.descriptor()),
            | _ => None,
        }
    }

    /// Whether a call designates directly a builtin spelled one of `names`.
    fn intrinsic_named(&self, callee: &'tu Expression<'tu>, names: &[&str]) -> bool {
        let Some(binding) = self.callee_binding(callee) else {
            return false;
        };
        let data = self.bindings[binding];
        !matches!(
            data.origin,
            super::super::BindingOrigin::User | super::super::BindingOrigin::ImplicitOrdinary
        ) && names.contains(&self.context.string_cache.at(data.name.name))
    }

    /// Whether the call's function is one Clang knows to return twice: one
    /// declared `returns_twice`, `__builtin_setjmp`, or with external linkage
    /// in a hosted unit `setjmp`, `_setjmp`, `sigsetjmp`, `__sigsetjmp`,
    /// `savectx`, `getcontext` or `vfork` (`ReturnsTwice` in `Builtins.td`).
    /// C99: §7.13.1.1, p. 243; PDF p. 255.
    fn callee_returns_twice(&mut self, callee: &'tu Expression<'tu>) -> bool {
        let Some(binding) = self.callee_binding(callee) else {
            return false;
        };
        let data = self.bindings[binding];
        if data.kind != BindingKind::Function {
            return false;
        }
        let name = self.context.string_cache.at(data.name.name);
        let builtin = name == "__builtin_setjmp";
        let library = data.linkage == Linkage::External
            && self.context.configuration.hosted()
            && matches!(
                name,
                "setjmp"
                    | "_setjmp"
                    | "sigsetjmp"
                    | "__sigsetjmp"
                    | "savectx"
                    | "getcontext"
                    | "vfork"
            );
        builtin || library || self.declared_returns_twice(binding)
    }

    /// The kind of the declaration marked `analyzer_noreturn` that the call
    /// names, if any (`CallExpr::getCalleeDecl`): Clang's graph ends the
    /// block at a direct call of such a function, as at one that does not
    /// return (`FunctionDecl::isAnalyzerNoReturn`), and the analysis forgets
    /// every value after a call through any such declaration.
    fn callee_analyzer_noreturn(&self, callee: &'tu Expression<'tu>) -> Option<BindingKind> {
        let binding = self.callee_binding(callee)?;
        self.functions
            .emission
            .analyzer_noreturn
            .contains_key(&binding)
            .then_some(self.bindings[binding].kind)
    }

    /// Whether the call's function is defined with an empty body
    /// (`FunctionDecl::hasTrivialBody`).
    fn trivial_callee(&self, callee: &'tu Expression<'tu>) -> bool {
        self.callee_binding(callee).is_some_and(|binding| {
            let data = self.bindings[binding];
            data.kind == BindingKind::Function
                && self.uninitialized.trivial.contains_key(&data.name.name)
        })
    }

    /// Whether an input operand with constraint `token` is read as a value,
    /// as Clang converts it unless only memory satisfies the constraint:
    /// `Some(true)` for a register, an immediate or `g`, `Some(false)` for
    /// memory alone, `None` for a constraint the builder does not read.
    /// GNU extension: GCC manual, "Simple Constraints".
    fn input_constraint_reads(
        &mut self,
        token: crate::translation_phases::preprocessing::Token,
    ) -> Option<bool> {
        let text = self.assembly_string(token)?;
        let text = text.trim_start_matches(['=', '+']);
        let letters = text.trim_start_matches(['%', '*', '&', '!', '?', ',', '#']);
        if letters.is_empty() {
            return None;
        }
        if letters
            .chars()
            .all(|c| matches!(c, 'm' | 'o' | 'V' | '<' | '>' | ','))
        {
            return Some(false);
        }
        if letters
            .chars()
            .all(|c| "rgXiInNsEFabcdSDAqQRlxyftuJKLMOGHe,".contains(c))
        {
            return Some(true);
        }
        None
    }
}

/// Whether `scalar` is an integer type: Clang's `isIntegralType`.
const fn scalar_is_integral(scalar: Scalar) -> bool {
    matches!(
        scalar,
        Scalar::Bool
            | Scalar::Char
            | Scalar::SignedChar
            | Scalar::UnsignedChar
            | Scalar::Short
            | Scalar::UnsignedShort
            | Scalar::Int
            | Scalar::UnsignedInt
            | Scalar::Long
            | Scalar::UnsignedLong
            | Scalar::LongLong
            | Scalar::UnsignedLongLong
            | Scalar::Int128
            | Scalar::UnsignedInt128
    )
}

/// Whether a list in `initializer` has a designator.
fn designated(initializer: &Initializer<'_>) -> bool {
    match initializer.kind {
        | InitializerType::AssignmentExpression(_) => false,
        | InitializerType::InitializerList(list) => list
            .elements
            .iter()
            .any(|element| element.designation.is_some() || designated(element.initializer)),
    }
}
