//! The bcc IR: the representation between semantic analysis and the back
//! ends. A [`Module`] holds functions, globals and interned call signatures in
//! one `'ir` arena. A defined [`Function`] has a [`Body`]: a control-flow
//! graph of blocks in SSA form, where blocks take typed parameters and every
//! branch passes matching arguments, in place of phi nodes. Each block holds
//! its instructions in program order; the entry block's parameters are the
//! function's parameters. Values are dense `u32` entities, instruction
//! records are 16 bytes, and longer operand lists live in a per-function
//! pool. Memory is explicit: stack slots, loads and stores ordered by their
//! position, and aggregates that are never values. The IR follows LLVM's
//! poison semantics without `undef`.
//!
//! A [`FunctionBuilder`] grows a body one instruction at a time.
//!
//! For `int add(int a, int b) { return a + b; }`, lowering declares `@add`
//! with the signature `(i32, i32) -> i32`, creates the entry block with
//! parameters `v0` and `v1`, inserts `v2 = iadd.i32 nsw v0, v1` and ends the
//! block with `return v2`:
//!
//! ```text
//! function @add(i32, i32) -> i32 external {
//! block0(v0: i32, v1: i32):
//!     v2 = iadd.i32 nsw v0, v1
//!     return v2
//! }
//! ```
//!
//! Read [`Module`], [`Function`], [`Body`] and [`InstData`] first, then
//! [`FunctionBuilder`] to see how bodies grow.
//!
//! - Representation: `entities.rs` defines the `u32` entities and their dense
//!   tables; `types.rs` the value types; `instructions.rs` the opcodes, the
//!   instruction record and its flags; `function.rs` a function body and its
//!   queries; `module.rs` signatures, globals, symbols and result types.
//! - Construction: `builder.rs` builds a body one instruction at a time.
//! - `tests.rs` and `tests/` exercise each of these.
//!
//! The IR is not a C translation phase. Where one of its rules exists because
//! of C, such as `nsw` for signed overflow, the item that states it cites the
//! clause.

// Representation
mod entities;
mod function;
mod instructions;
mod module;
mod types;

// Construction
mod builder;

pub(crate) use builder::FunctionBuilder;
pub(crate) use entities::{
    AccessTag,
    Block,
    ConstId,
    Entity,
    FuncId,
    GlobalId,
    Inst,
    SigId,
    StackSlot,
    Table,
    Value,
};
pub(crate) use function::{
    Body,
    StackSlotData,
    ValueData,
    ValueDef,
};
pub(crate) use instructions::{
    Align,
    Bits64,
    BlockCall,
    CaseList,
    FloatCC,
    InstData,
    InstFlags,
    IntCC,
    MemFlags,
    Opcode,
    ValueList,
};
pub(crate) use module::{
    Global,
    GlobalDecl,
    GlobalInit,
    Linkage,
    Relocation,
    Signature,
    Symbol,
};
pub(crate) use types::Type;

use crate::util::bump::{
    ArenaMap,
    Bump,
};

/// A translation unit's IR: its functions, globals and interned signatures,
/// and the target they are for. Everything it holds lives in its `'ir` arena.
pub(crate) struct Module<'ir> {
    arena:         &'ir Bump,
    triple:        &'ir str,
    data_layout:   &'ir str,
    functions:     Table<'ir, FuncId, Function<'ir>>,
    globals:       Table<'ir, GlobalId, Global<'ir>>,
    signatures:    Table<'ir, SigId, Signature<'ir>>,
    signature_ids: ArenaMap<'ir, Signature<'ir>, SigId>,
    /// Functions and globals share one namespace.
    symbols:       ArenaMap<'ir, &'ir str, Symbol>,
}

/// A function of a module. It is defined if it has a body.
#[derive(Debug)]
pub(crate) struct Function<'ir> {
    pub(crate) name:      &'ir str,
    pub(crate) signature: SigId,
    pub(crate) linkage:   Linkage,
    pub(crate) body:      Option<Body<'ir>>,
}

// Tests
#[cfg(test)]
#[expect(
    clippy::disallowed_types,
    clippy::disallowed_macros,
    clippy::disallowed_methods,
    reason = "Tests build inputs and expected values with std types; the arena rule covers the \
              compiler, not its tests."
)]
mod tests;
