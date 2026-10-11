//! The vocabulary of the per-function loop: the tasks on its work stack and
//! the operands they pass on the operand stack.
//!
//! A task that needs its children evaluated first pushes a continuation and
//! then the children, so the children run first; each child leaves one
//! [`Item`] on the operand stack. Statements leave nothing.

use super::ssa::Var;
use crate::{
    ir::{
        Block,
        FuncId,
        StackSlot,
        Value,
    },
    translation_phases::{
        parsing::{
            declaration_syntax::{
                Declaration,
                Initializer,
            },
            syntax::{
                ConditionalExpression,
                Expression,
                Statement,
            },
        },
        semantic_analysis::TypeId,
    },
};

/// One unit of work on the function's explicit stack.
#[derive(Clone, Copy, Debug)]
pub(super) enum Task<'tu> {
    // Statements
    Statement(&'tu Statement<'tu>),
    Declaration(&'tu Declaration<'tu>),
    /// Enter `block`: it is sealed if all its predecessors are known.
    Enter {
        block: Block,
        seal:  bool,
    },
    /// Jump from the current block to `block`, then seal it if asked.
    JumpTo {
        block: Block,
        seal:  bool,
    },
    Seal(Block),
    /// Push break and continue targets.
    PushTargets(JumpTargets),
    PopTargets,
    /// Pop the operand and return it.
    Return,
    /// Pop the switch's controlling value and dispatch on it.
    Switch {
        statement: &'tu Statement<'tu>,
        body:      &'tu Statement<'tu>,
        exit:      Block,
    },
    /// Close a switch: jump to its exit and seal its case blocks.
    EndSwitch {
        statement: &'tu Statement<'tu>,
        exit:      Block,
    },
    /// Store the initializer value on top of the stack into an object being
    /// initialized, at `offset` bytes from its start.
    Initialize {
        object: Object,
        offset: u64,
        ty:     TypeId,
    },
    /// Copy a string literal into a character array being initialized.
    InitializeString {
        object:  Object,
        offset:  u64,
        size:    u64,
        literal: &'tu Expression<'tu>,
    },
    /// Initialize an automatic object of type `ty` from a braced list or an
    /// aggregate expression.
    InitializeAggregate {
        object:      Object,
        ty:          TypeId,
        initializer: &'tu Initializer<'tu>,
    },
    /// Push the compound literal in `slot`, now initialized, as a place.
    PushSlot {
        slot: StackSlot,
        ty:   TypeId,
    },

    // Expressions
    /// Evaluate an expression and apply the conversions its parent recorded
    /// on it.
    Evaluate(&'tu Expression<'tu>),
    /// Evaluate an expression without its conversions, leaving a place for
    /// an lvalue.
    EvaluateRaw(&'tu Expression<'tu>),
    /// Apply the conversions recorded on an expression, from the `skip`th.
    Convert {
        expression: &'tu Expression<'tu>,
        skip:       usize,
    },
    /// Complete an operator whose operands are on the stack.
    Finish(&'tu Expression<'tu>),
    /// Copy the place on top of the stack and convert the copy, as a
    /// compound assignment or increment reads its operand.
    ReadForUpdate(&'tu Expression<'tu>),
    /// Pop an operand that is not needed.
    Discard,
    /// Evaluate `expression` as a condition: branch to `then` if it is
    /// nonzero, else to `otherwise`.
    Condition {
        expression: &'tu Expression<'tu>,
        then:       Block,
        otherwise:  Block,
    },
    /// Pop an operand and branch on whether it is nonzero.
    Branch {
        then:      Block,
        otherwise: Block,
    },
    /// After the left operand of `&&` or `||`: branch to the right operand
    /// or straight to `merge` with the result.
    Logical {
        and:   bool,
        right: Block,
        merge: Block,
    },
    /// After either operand of `&&` or `||` or an arm of `?:`: pass the
    /// value to `merge`.
    JumpWithValue {
        merge: Block,
        truth: bool,
    },
    /// Continue after a value-producing merge.
    Merge {
        merge: Block,
        ty:    TypeId,
        truth: bool,
    },
    /// Evaluate the arms of a conditional once its condition has branched.
    ConditionalArms {
        conditional: &'tu ConditionalExpression<'tu>,
        then:        Block,
        otherwise:   Block,
        merge:       Block,
        ty:          TypeId,
    },
}

/// A value on the operand stack with its C type.
#[derive(Clone, Copy, Debug)]
pub(super) struct Item {
    pub(super) operand: Operand,
    pub(super) ty:      TypeId,
}

/// What an evaluated expression is.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Operand {
    Void,
    Value(Value),
    /// An `int` that is 0 or 1, still held as the `i1` of a comparison, so a
    /// condition can branch on it directly.
    Truth(Value),
    /// An lvalue or function designator.
    Place(Place),
    /// A structure or union rvalue, by the address of an object holding it.
    Aggregate(Value),
}

/// Where an lvalue is.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Place {
    Variable(Var),
    Memory(Value),
    Function(FuncId),
}

/// An automatic object an initializer fills: a declared one, or the
/// unnamed object of a compound literal.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Object {
    Local(usize),
    Slot(StackSlot),
}

/// The storage of an automatic object.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Local {
    Variable(Var),
    Slot(StackSlot),
}

/// Where `break` and `continue` go from inside a loop or switch.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) struct JumpTargets {
    pub(super) break_to:    Block,
    pub(super) continue_to: Option<Block>,
}
