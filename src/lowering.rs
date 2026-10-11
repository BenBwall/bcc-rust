//! Lowering translates the analyzed syntax tree into the bcc IR. It reads
//! the syntax, the expression records and conversion runs semantic analysis
//! retained, and the target's data model, and makes every C rule explicit:
//! each conversion becomes an instruction, each lvalue a load or an SSA
//! read, each control structure blocks and branches.
//!
//! [`lower_translation_unit`] refuses a unit with errors, declares every
//! defined function and file-scope object, and then lowers one function at a
//! time with [`FunctionLowerer::run`]. That loop pops [`Task`]s from an
//! explicit work stack, never recursing over syntax: a statement pushes its
//! parts, an expression pushes a continuation and then its operands, and
//! each evaluated operand leaves an [`Item`] on an operand stack. Scalar
//! locals whose address is never taken are SSA variables built with Braun et
//! al.'s algorithm; the others, and every array and structure, live in stack
//! slots. A construct outside the prototype subset stops its function with a
//! [`LoweringError`] at that syntax, never with a panic or wrong code.
//!
//! For `int f(int a) { return a + 1; }`, the entry block's parameter defines
//! the variable `a`. The return statement evaluates `a + 1`: reading `a`
//! applies its lvalue conversion, a read of the variable, then the addition
//! emits `iadd.i32 nsw` because signed overflow is undefined. The `return`
//! applies the conversion to the result type and ends the block.
//!
//! Read [`lower_translation_unit`], [`FunctionLowerer::run`] and
//! [`FunctionLowerer::step`], then `work.rs` for the task vocabulary,
//! `ssa.rs` for the function under construction and `emit.rs` for how it
//! becomes IR.
//!
//! Files by role:
//! - Translation unit: `module_items.rs` declares functions and globals and
//!   lays out static initializers; `address_taken.rs` finds the locals that
//!   need memory.
//! - Function bodies: `work.rs` defines tasks and operands; `statements.rs`
//!   lowers statements and declarations; `expressions.rs` operators, calls and
//!   conversions; `places.rs` lvalues, loads and stores; `initializers.rs`
//!   walks braced initializers.
//! - Construction: `ssa.rs` holds the draft function and its SSA variables;
//!   `emit.rs` copies a finished draft into the module; `types.rs` maps C types
//!   to IR types.
//! - Errors: `errors.rs`. Tests: `tests.rs` and `tests/`.
//!
//! C99: §5.1.2.3, pp. 13-15; PDF pp. 25-27 (program execution: lowering
//! keeps the abstract machine's side effects and sequence points).
//! C99: §6.3, pp. 42-48; PDF pp. 54-60 (conversions); §6.5, pp. 67-94;
//! PDF pp. 79-106 (expressions); §6.8, pp. 131-139; PDF pp. 143-151
//! (statements); §6.9, pp. 140-143; PDF pp. 152-155 (external definitions).

// Translation unit
mod address_taken;
mod module_items;

// Function bodies
mod expressions;
mod initializers;
mod places;
mod statements;
mod work;

// Construction
mod emit;
mod ssa;
mod types;

// Errors
mod errors;

use std::fmt;

pub(crate) use errors::{
    Construct,
    LoweringError,
    LoweringErrorKind,
    LoweringErrors,
};
use module_items::Unit;
use ssa::Draft;
use work::{
    Item,
    JumpTargets,
    Local,
    Operand,
    Place,
    Task,
};

use crate::{
    ir::{
        Block,
        FuncId,
        InstData,
        Module,
        Profile,
        Type,
        verify_function,
    },
    target::Target,
    translation_phases::{
        Context,
        parsing::{
            ParsedTranslationUnit,
            syntax::ExpressionType,
        },
        semantic_analysis::{
            FunctionRecord,
            SemanticTranslationUnit,
            TypeId,
        },
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

/// Lowers a translation unit to an IR module for `target`, allocated in
/// `arena`. `scratch` holds each function's working state and is reset
/// between functions. A unit that any phase reported an error in is
/// refused; any construct outside the supported subset is reported with its
/// provenance, after every function has been attempted.
/// C99: §5.1.1.3 paragraph 1, p. 11; PDF p. 23.
pub(crate) fn lower_translation_unit<'tu, 'ir>(
    context: &Context<'tu>,
    syntax: &ParsedTranslationUnit<'tu>,
    sema: &SemanticTranslationUnit<'tu>,
    target: Target,
    arena: &'ir Bump,
    scratch: &mut Bump,
) -> Result<Module<'ir>, LoweringErrors<'ir>> {
    let mut errors = ArenaVec::new_in(arena);
    if !sema.lowerable(context) {
        errors.push(LoweringError {
            kind:   LoweringErrorKind::UnitHasErrors,
            source: None,
        });
        return Err(LoweringErrors {
            errors: arena.alloc_slice_copy(&errors),
        });
    }
    let tables = Bump::new();
    let mut unit = Unit::new(context, sema, target, arena, &tables);
    let functions = unit.declare_items(syntax, &mut errors);
    for record in functions {
        scratch.reset();
        let Some(func) = record.1 else {
            continue;
        };
        if let Err(error) = FunctionLowerer::lower(&mut unit, record.0, func, scratch) {
            errors.push(error);
        }
    }
    if errors.is_empty() {
        Ok(unit.into_module())
    } else {
        Err(LoweringErrors {
            errors: arena.alloc_slice_copy(&errors),
        })
    }
}

impl<'u, 'a, 'tu, 'ir, 's> FunctionLowerer<'u, 'a, 'tu, 'ir, 's> {
    /// Lowers one function definition and installs its body.
    fn lower(
        unit: &'u mut Unit<'a, 'tu, 'ir>,
        record: &'tu FunctionRecord<'tu>,
        func: FuncId,
        scratch: &'s Bump,
    ) -> Result<(), LoweringError> {
        let mut lowerer = Self::new(unit, record, func, scratch)?;
        lowerer.run()?;
        lowerer.finish_body()?;
        let FunctionLowerer { unit, draft, .. } = lowerer;
        emit::emit(draft, &mut unit.module, func)?;
        if cfg!(debug_assertions) {
            let errors = verify_function(&unit.module, func, Profile::PreAbi, scratch);
            assert!(
                errors.is_empty(),
                "lowering produced invalid IR:\n{}\n{}",
                fmt::from_fn(|f| errors.iter().try_for_each(|error| writeln!(f, "{error}"))),
                unit.module.display_function(func),
            );
        }
        Ok(())
    }

    /// Pops and performs tasks until the body is lowered.
    fn run(&mut self) -> Result<(), LoweringError> {
        while let Some(task) = self.tasks.pop() {
            self.step(task)?;
        }
        debug_assert!(self.operands.is_empty(), "every operand is consumed");
        Ok(())
    }

    /// Performs one task, pushing any that follow from it.
    fn step(&mut self, task: Task<'tu>) -> Result<(), LoweringError> {
        match task {
            | Task::Statement(statement) => self.statement(statement),
            | Task::Declaration(declaration) => self.declaration(declaration),
            | Task::Enter { block, seal } => {
                if seal {
                    self.draft.seal(block);
                }
                self.draft.switch_to(block);
                Ok(())
            },
            | Task::JumpTo { block, seal } => {
                self.draft.jump(block, &[]);
                if seal {
                    self.draft.seal(block);
                }
                Ok(())
            },
            | Task::Seal(block) => {
                self.draft.seal(block);
                Ok(())
            },
            | Task::PushTargets(targets) => {
                self.jumps.push(targets);
                Ok(())
            },
            | Task::PopTargets => {
                _ = self.jumps.pop();
                Ok(())
            },
            | Task::Return => self.return_value(),
            | Task::Switch {
                statement,
                body,
                exit,
            } => self.switch(statement, body, exit),
            | Task::EndSwitch { statement, exit } => {
                self.end_switch(statement, exit);
                Ok(())
            },
            | Task::Initialize { object, offset, ty } => self.initialize(object, offset, ty),
            | Task::InitializeString {
                object,
                offset,
                size,
                literal,
            } => self.initialize_string(object, offset, size, literal),
            | Task::InitializeAggregate {
                object,
                ty,
                initializer,
            } => self.initialize_aggregate(object, ty, initializer),
            | Task::PushSlot { slot, ty } => {
                let address = self.inst(InstData::StackAddr { slot }, Type::Ptr);
                self.operands.push(Item {
                    operand: Operand::Place(Place::Memory(address)),
                    ty,
                });
                Ok(())
            },
            | Task::Evaluate(expression) => {
                // A compound assignment's first conversion is its own store
                // back to the left operand's type.
                let skip = match expression.kind {
                    | ExpressionType::Binary { operator, .. }
                        if expressions::compound_operator(operator).is_some() =>
                        1,
                    | _ => 0,
                };
                self.tasks.push(Task::Convert { expression, skip });
                self.evaluate_raw(expression)
            },
            | Task::EvaluateRaw(expression) => self.evaluate_raw(expression),
            | Task::Convert { expression, skip } => self.convert_recorded(expression, skip),
            | Task::Finish(expression) => self.finish(expression),
            | Task::ReadForUpdate(expression) => {
                let place = *self.operands.last().expect("the place is on the stack");
                self.operands.push(place);
                self.convert_recorded(expression, 0)
            },
            | Task::Discard => {
                _ = self.pop();
                Ok(())
            },
            | Task::Condition {
                expression,
                then,
                otherwise,
            } => self.condition(expression, then, otherwise),
            | Task::Branch { then, otherwise } => self.branch(then, otherwise),
            | Task::Logical { and, right, merge } => self.logical(and, right, merge),
            | Task::JumpWithValue { merge, truth } => self.jump_with_value(merge, truth),
            | Task::Merge { merge, ty, truth } => {
                self.merge(merge, ty, truth);
                Ok(())
            },
            | Task::ConditionalArms {
                conditional,
                then,
                otherwise,
                merge,
                ty,
            } => {
                self.conditional_arms(conditional, then, otherwise, merge, ty);
                Ok(())
            },
        }
    }
}

/// The state of lowering one function body.
struct FunctionLowerer<'u, 'a, 'tu, 'ir, 's> {
    unit:          &'u mut Unit<'a, 'tu, 'ir>,
    record:        &'tu FunctionRecord<'tu>,
    /// The function's name, for naming its static locals.
    name:          &'a str,
    draft:         Draft<'s>,
    scratch:       &'s Bump,
    tasks:         ArenaVec<'s, Task<'tu>>,
    operands:      ArenaVec<'s, Item>,
    /// The storage of each automatic object, by binding.
    locals:        ArenaMap<'s, usize, Local>,
    /// The block of each label, created at its first `goto` or definition.
    labels:        ArenaMap<'s, StringCacheId, Block>,
    /// The block of each `case` or `default` statement, by its address.
    cases:         ArenaMap<'s, usize, Block>,
    jumps:         ArenaVec<'s, JumpTargets>,
    /// The case blocks of the open switches, sealed when each ends.
    switch_blocks: ArenaVec<'s, Block>,
    /// Where each open switch's blocks start in `switch_blocks`.
    switch_starts: ArenaVec<'s, usize>,
    /// The unqualified result type that `return` converts to.
    result:        TypeId,
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
pub(crate) mod tests;
