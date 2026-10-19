//! The bcc IR: the representation between semantic analysis and the back
//! ends. A [`Module`] holds functions, globals and interned call signatures in
//! one `'ir` arena. A defined [`Function`] has a [`Body`]: a control-flow
//! graph of blocks in SSA form, where blocks take typed parameters and every
//! branch passes matching arguments, in place of phi nodes. Each block holds
//! its instructions in program order; the entry block's parameters are the
//! function's parameters. Values are dense `u32` entities, instruction
//! records are 16 bytes, and longer operand lists live in a per-function
//! pool. Memory is explicit: stack slots, loads and stores ordered by their
//! position, and aggregates that are never values. Address-taken blocks have
//! labels, which `block_addr` and relocations name and an `indirect_br`
//! targets, so renumbering never invalidates a block address. The IR follows
//! LLVM's poison semantics without `undef`.
//!
//! A [`FunctionBuilder`] appends instructions, [`Module`]'s `Display` prints
//! the textual form, [`parse_module`] reads it back, [`verify_module`] checks
//! the invariants passes rely on, and [`DominatorTree`] computes dominance
//! over a [`ControlFlowGraph`].
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
//! [`FunctionBuilder`] to see how bodies grow, and `verifier.rs` for the rules
//! every body keeps.
//!
//! - Representation: `entities.rs` defines the `u32` entities and their dense
//!   tables; `types.rs` the value types; `instructions.rs` the opcodes, the
//!   instruction record and its flags; `function.rs` a function body and its
//!   queries; `module.rs` signatures, globals, symbols and result types.
//! - Construction: `builder.rs` builds a body one instruction at a time.
//! - Textual form: `printer.rs` writes it; `parser.rs` reads it, with
//!   `parser/lexer.rs` splitting it into tokens.
//! - Analysis and checking: `cfg.rs` computes predecessors and successors;
//!   `dominators.rs` reverse post-order and the dominator tree; `verifier.rs`
//!   checks a module and reports structured errors.
//! - `tests.rs` and `tests/` exercise each of these.
//!
//! The IR is not a C translation phase. Where one of its rules exists because
//! of C, such as `nsw` for signed overflow, the item that states it cites the
//! clause.

// Representation
mod bits;
mod entities;
mod function;
mod instructions;
mod module;
mod types;

// Construction
mod builder;

// Textual form
mod parser;
mod printer;

// Analysis and checking
mod cfg;
mod dominators;
mod verifier;

pub(crate) use bits::{
    bit_unary,
    unsigned_overflow,
};
pub(crate) use builder::FunctionBuilder;
pub(crate) use cfg::ControlFlowGraph;
pub(crate) use dominators::DominatorTree;
pub(crate) use entities::{
    AccessTag,
    Block,
    BlockLabel,
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
    AtomicOp,
    Bits64,
    BlockCall,
    BlockList,
    CaseList,
    FloatCC,
    InstData,
    InstFlags,
    IntCC,
    MemFlags,
    MemoryOrder,
    Opcode,
    ValueList,
};
pub(crate) use module::{
    AggregateFlags,
    AggregateLayout,
    AggregateParam,
    AggregateScalar,
    DllStorage,
    ExtendedParam,
    Extension,
    FirstMember,
    Global,
    GlobalDecl,
    GlobalInit,
    InitFunction,
    InitKind,
    InlineControl,
    Linkage,
    ParamAbi,
    Relocation,
    RelocationTarget,
    ResultRegisters,
    Signature,
    Symbol,
    SymbolAttributes,
    Visibility,
    valid_section_name,
};
pub(crate) use parser::{
    ParseError,
    ParseErrorKind,
    parse_module,
};
pub(crate) use types::Type;
pub(crate) use verifier::{
    EntityKind,
    Location,
    Profile,
    VerifierError,
    VerifierErrorKind,
    verify_function,
    verify_module,
};

use crate::util::bump::{
    ArenaMap,
    ArenaVec,
    Bump,
};

/// A translation unit's IR: its functions, globals and interned signatures,
/// and the target they are for. Everything it holds lives in its `'ir` arena.
pub(crate) struct Module<'ir> {
    arena:          &'ir Bump,
    triple:         &'ir str,
    data_layout:    &'ir str,
    /// Whether ABI lowering has run.
    profile:        Profile,
    functions:      Table<'ir, FuncId, Function<'ir>>,
    globals:        Table<'ir, GlobalId, Global<'ir>>,
    signatures:     Table<'ir, SigId, Signature<'ir>>,
    signature_ids:  ArenaMap<'ir, Signature<'ir>, SigId>,
    /// Functions and globals share one namespace.
    symbols:        ArenaMap<'ir, &'ir str, Symbol>,
    /// Functions run before or after the program's entry, in this order of
    /// registration; back ends sort them by priority.
    init_functions: ArenaVec<'ir, InitFunction>,
}

/// A function of a module. It is defined if it has a body.
#[derive(Debug)]
pub(crate) struct Function<'ir> {
    pub(crate) name:           &'ir str,
    pub(crate) signature:      SigId,
    pub(crate) linkage:        Linkage,
    pub(crate) body:           Option<Body<'ir>>,
    /// Alias target, section and retention requests.
    pub(crate) attributes:     SymbolAttributes<'ir>,
    /// A requested minimum alignment of the function's code.
    pub(crate) align:          Option<Align>,
    /// A call may return a second time, after a `longjmp` restores the
    /// environment it saved (`setjmp`, `vfork`, GNU `returns_twice`). The
    /// optimizer does not inline across such frames, the LLVM back end
    /// marks the declaration and its calls `returns_twice`, and the native
    /// back end refuses them. C99: §7.13.2.1 paragraph 4, p. 244; PDF p.
    /// 256.
    pub(crate) returns_twice:  bool,
    /// Whether calls may, must or must not be expanded inline.
    pub(crate) inlining:       InlineControl,
    /// For the private copy of an `always_inline` inline-only definition,
    /// the external function it copies. Direct calls name the copy so that
    /// bcc's optimizer, interpreter and native back end expand or run it;
    /// the LLVM back end instead emits the body as that function's
    /// `available_externally` definition and redirects the copy's uses to
    /// it, as Clang does, so LLVM's inliner decides as for Clang (a
    /// recursive body is not inline viable and calls the external
    /// definition). C99: §6.7.4p6, pp. 112-113; PDF pp. 124-125.
    pub(crate) inline_copy_of: Option<&'ir str>,
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
