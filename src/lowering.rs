//! Lowering translates the analyzed syntax tree into the bcc IR. It reads
//! the syntax, the expression records and conversion runs semantic analysis
//! retained, and the target's data model, and makes every C rule explicit:
//! each conversion becomes an instruction, each lvalue a load or an SSA
//! read, each control structure blocks and branches.
//!
//! [`lower_with_sources`] refuses a unit with errors, declares every
//! defined function and file-scope object, and then lowers one function at a
//! time with [`FunctionLowerer::run`]. That loop pops [`Task`]s from an
//! explicit work stack, never recursing over syntax: a statement pushes its
//! parts, an expression pushes a continuation and then its operands, and
//! each evaluated operand leaves an [`Item`] on an operand stack. Scalar
//! locals whose address is never taken are SSA variables built with Braun et
//! al.'s algorithm; the others, and every array and structure, live in stack
//! slots. A structure or union passed or returned by value travels through a
//! `ptr` that the pre-ABI signature describes: the caller passes a fresh copy
//! of each such argument and a fresh object for such a result, and ABI
//! lowering later decides how the target passes them. A construct outside
//! the prototype subset stops its function with a
//! [`LoweringError`] at that syntax, never with a panic or wrong code.
//!
//! For `int f(int a) { return a + 1; }`, the entry block's parameter defines
//! the variable `a`. The return statement evaluates `a + 1`: reading `a`
//! applies its lvalue conversion, a read of the variable, then the addition
//! emits `iadd.i32 nsw` because signed overflow is undefined. The `return`
//! applies the conversion to the result type and ends the block.
//!
//! Read [`lower_with_sources`], [`FunctionLowerer::run`] and
//! [`FunctionLowerer::step`], then `work.rs` for the task vocabulary,
//! `ssa.rs` for the function under construction and `emit.rs` for how it
//! becomes IR.
//!
//! Files by role:
//! - Translation unit: `module_items.rs` declares functions and globals and
//!   lays out static initializers; `address_taken.rs` finds the locals that
//!   need memory; `extensions.rs` reports retained attribute refusals and
//!   queries decoded symbol requests; `symbols.rs` maps retained entity
//!   attributes to weak linkage, aliases, sections and constructor/destructor
//!   lists.
//! - Function bodies: `work.rs` defines tasks and operands; `statements.rs`
//!   lowers statements and declarations; `expressions.rs` operators, calls and
//!   conversions, including the floating ones; `places.rs` lvalues, loads and
//!   stores; `alignment.rs` owns backing-place guarantees and all memory and
//!   allocation alignment; `bit_fields.rs` bit-field reads and writes;
//!   `initializers.rs` walks braced initializers; `vla.rs` evaluates saved
//!   array extents and runs the cleanups of the regions an exit leaves: dynamic
//!   stack restores, and GNU cleanup calls from `cleanup.rs`.
//! - Construction: `ssa.rs` holds the draft function and its SSA variables;
//!   `emit.rs` copies a finished draft into the module; `types.rs` maps C types
//!   to IR types; shared `crate::float_parsing::bits` turns folded floating
//!   constants into IR bits.
//! - Errors: `errors.rs`. Tests: `tests.rs` and `tests/`.
//!
//! C99: §5.1.2.3, pp. 13-15; PDF pp. 25-27 (program execution: lowering
//! keeps the abstract machine's side effects and sequence points).
//! C99: §6.3, pp. 42-48; PDF pp. 54-60 (conversions); §6.5, pp. 67-94;
//! PDF pp. 79-106 (expressions); §6.8, pp. 131-139; PDF pp. 143-151
//! (statements); §6.9, pp. 140-143; PDF pp. 152-155 (external definitions).

// Translation unit
mod address_taken;
mod extensions;
mod module_items;
mod symbols;

// Function bodies
mod alignment;
mod bit_fields;
mod builtin_effects;
mod builtins;
mod cleanup;
mod complex;
mod expressions;
mod gnu;
mod initializers;
mod places;
mod statements;
mod vla;
mod work;

// Construction
mod emit;
mod ssa;
mod types;

// Diagnostics
mod call_diagnostics;
mod errors;

use std::fmt;

pub(crate) use call_diagnostics::{
    CallDiagnostic,
    retained_call_diagnostics,
};
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
    StatementMode,
    Task,
};

use crate::{
    float_parsing::bits as floats,
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
        SourceVectors,
        parsing::{
            ParsedTranslationUnit,
            syntax::ExpressionType,
        },
        semantic_analysis::{
            FunctionRecord,
            LabelId,
            RegionId,
            SemanticTranslationUnit,
            StatementId,
            TypeId,
        },
    },
    util::bump::{
        ArenaMap,
        ArenaVec,
        Bump,
    },
};

/// Panic message for entity queries that cannot miss. `Unit::new` refuses a
/// unit whose policy-bearing bindings (functions and linked or static
/// objects) lack entity facts, and lowering only queries those bindings.
const ENTITY_FACTS: &str = "entity facts checked by Unit::new: every function and linked or \
                            static object binding has retained entity facts";

/// Lowers a translation unit to an IR module for `target`, allocated in
/// `arena`. `scratch` holds each function's working state and is reset
/// between functions. A unit that any phase reported an error in is
/// refused; any construct outside the supported subset is reported with its
/// provenance, after every function has been attempted. With
/// `plain_inline`, plain inline-only bodies are lowered as copies for LLVM's
/// optimizing pipelines ([`Unit::lower_plain_inline_bodies`]).
/// C99: §5.1.1.3 paragraph 1, p. 11; PDF p. 23.
pub(crate) fn lower_with_sources<'tu, 'ir>(
    context: &Context<'tu>,
    syntax: &ParsedTranslationUnit<'tu>,
    sema: &SemanticTranslationUnit<'tu>,
    target: Target,
    arena: &'ir Bump,
    scratch: &mut Bump,
    plain_inline: bool,
) -> Result<LoweredUnit<'ir>, LoweringErrors<'ir>> {
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
    match lower_unit(context, syntax, sema, target, arena, scratch, plain_inline) {
        // A plain inline-only body that lowering refuses is dropped on its
        // own, and so is one whose attribute refusals would apply. Other
        // errors that only such bodies cause, such as an object they alone
        // reach, are rarer: this backstop lowers the unit again without
        // any, so that they never refuse it (C99 §6.7.4p6).
        | (Err(_), true) => lower_unit(context, syntax, sema, target, arena, scratch, false).0,
        | (result, _) => result,
    }
}

/// [`lower_with_sources`], with the plain inline-only bodies when
/// `plain_inline` is set; also returns whether there was any.
fn lower_unit<'tu, 'ir>(
    context: &Context<'tu>,
    syntax: &ParsedTranslationUnit<'tu>,
    sema: &SemanticTranslationUnit<'tu>,
    target: Target,
    arena: &'ir Bump,
    scratch: &mut Bump,
    plain_inline: bool,
) -> (Result<LoweredUnit<'ir>, LoweringErrors<'ir>>, bool) {
    let mut errors = ArenaVec::new_in(arena);
    let tables = Bump::new();
    let mut unit = match Unit::new(context, sema, target, arena, &tables) {
        | Ok(unit) => unit,
        | Err(error) => {
            let errors = arena.alloc_slice_copy(&[error]);
            return (Err(LoweringErrors { errors }), false);
        },
    };
    let plain_inline = plain_inline && unit.lower_plain_inline_bodies();
    let functions = unit.declare_items(syntax, &mut errors);
    let mut function_sources = ArenaVec::new_in(arena);
    for record in functions {
        scratch.reset();
        let Some(func) = record.1 else {
            continue;
        };
        // Record the binding-to-emitted-symbol association at its owner,
        // including assembler aliases. ABI diagnostics need no syntax walk
        // or spelling-based reconstruction. C99 §5.1.1.3p1, p. 11; PDF p. 23.
        if let Some(binding) = record.0.binding {
            function_sources.push((
                unit.module.function(func).name,
                sema.bindings[binding].name.source_vectors,
            ));
        }
        if let Err(error) = FunctionLowerer::lower(&mut unit, record.0, func, scratch) {
            // A plain inline-only body is optional: one that lowering
            // refuses is not emitted, and calls keep reaching the external
            // definition (C99 §6.7.4p6). Its copy stays bodiless.
            // The module planner already reports a local static it could not
            // define; the body declaring it refuses with the same error.
            if !record
                .0
                .binding
                .is_some_and(|binding| unit.optional_inline_body(binding))
                && !errors.contains(&error)
            {
                errors.push(error);
            }
        }
    }
    let result = if errors.is_empty() {
        let call_diagnostics = arena.alloc_slice_copy(&unit.call_diagnostics);
        Ok(LoweredUnit {
            call_diagnostics,
            module: unit.into_module(),
            function_sources: arena.alloc_slice_copy(&function_sources),
        })
    } else {
        Err(LoweringErrors {
            errors: arena.alloc_slice_copy(&errors),
        })
    };
    (result, plain_inline)
}

/// IR and the source owners recorded while its function definitions are
/// projected from semantic bindings. Raw textual IR remains source-free.
/// C99: §5.1.1.3p1, p. 11; PDF p. 23.
pub(crate) struct LoweredUnit<'ir> {
    pub(crate) module:           Module<'ir>,
    pub(crate) call_diagnostics: &'ir [CallDiagnostic],
    pub(crate) function_sources: &'ir [(&'ir str, SourceVectors)],
}

/// Test adapter for the pre-ABI module seam.
#[cfg(test)]
pub(crate) fn lower_translation_unit<'tu, 'ir>(
    context: &Context<'tu>,
    syntax: &ParsedTranslationUnit<'tu>,
    sema: &SemanticTranslationUnit<'tu>,
    target: Target,
    arena: &'ir Bump,
    scratch: &mut Bump,
) -> Result<Module<'ir>, LoweringErrors<'ir>> {
    lower_with_sources(context, syntax, sema, target, arena, scratch, true).map(|unit| unit.module)
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
        emit::emit(draft, &mut unit.module, func, &mut unit.call_diagnostics)?;
        unit.defines_symbol(crate::ir::Symbol::Function(func));
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
            | Task::Statement(statement) => self.statement(statement, StatementMode::Discard),
            | Task::StatementValue(statement) => self.statement(statement, StatementMode::Value),
            | Task::StatementEffects(statement) =>
                self.statement(statement, StatementMode::Effects),
            | Task::Declaration(declaration) => self.declaration(declaration),
            | Task::DeclareBinding(binding, initializer, source) =>
                self.declare_binding(binding, initializer, source),
            | Task::FinishBound(index) => self.finish_bound(index),
            | Task::Sizeof(object, result, source) => {
                let size = self.emit_size_formula(object, source)?;
                let ty = self.value_type(result, source)?;
                let size = self.extend(size, false, ty);
                self.push_item(Operand::Value(size), result);
                Ok(())
            },
            | Task::CleanupNormal(statement) => self.cleanup_normal(statement),
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
            | Task::Return(statement) => self.return_value(statement),
            | Task::IndirectJump(statement) => self.indirect_jump(statement),
            | Task::Switch { statement, exit } => self.switch(statement, exit),
            | Task::EndSwitch { statement, exit } => {
                self.end_switch(statement, exit);
                Ok(())
            },
            | Task::Initialize {
                object,
                offset,
                ty,
                bit_field,
            } => self.initialize(object, offset, ty, bit_field),
            | Task::InitializeZero {
                object,
                offset,
                size,
                ty,
                mask,
            } => self.initialize_zero(object, offset, size, ty, mask),
            | Task::InitializeCopies {
                object,
                offset,
                size,
                stride,
                count,
                zero,
            } => self.initialize_copies(object, offset, size, stride, count, zero),
            | Task::InitializeString {
                object,
                offset,
                size,
                literal,
                ty,
            } => self.initialize_string(object, offset, size, literal, ty),
            | Task::InitializeAggregate {
                object,
                ty,
                initializer,
            } => self.initialize_aggregate(object, ty, initializer),
            | Task::RememberInitializer(index) => {
                self.initializer_values[index] = *self
                    .operands
                    .last()
                    .expect("an initializer value is on the stack");
                Ok(())
            },
            | Task::ReuseInitializer(index) => {
                self.operands.push(self.initializer_values[index]);
                Ok(())
            },
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
            | Task::BuiltinEffects(expression, context) =>
                self.builtin_effects(expression, context),
            | Task::SnapshotArgument(expression) => self.snapshot_argument(expression),
            | Task::Convert { expression, skip } => self.convert_recorded(expression, skip),
            | Task::Finish(expression) => self.finish(expression),
            | Task::FinishBuiltin(expression, builtin) => self.finish_builtin(expression, builtin),
            | Task::OmittedConditional {
                expression,
                conditional,
                otherwise,
                merge,
            } => self.omitted_conditional(expression, conditional, otherwise, merge),
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
    unit:               &'u mut Unit<'a, 'tu, 'ir>,
    record:             &'tu FunctionRecord<'tu>,
    /// The function's name, for naming its static locals.
    name:               &'a str,
    draft:              Draft<'s>,
    scratch:            &'s Bump,
    tasks:              ArenaVec<'s, Task<'tu>>,
    operands:           ArenaVec<'s, Item>,
    complex_values:     ArenaVec<'s, [crate::ir::Value; 2]>,
    initializer_values: ArenaVec<'s, Item>,
    /// The guaranteed alignment of each known backing address.
    alignments:         ArenaMap<'s, crate::ir::Value, crate::ir::Align>,
    /// The storage of each automatic object, by binding.
    locals:             ArenaMap<'s, usize, Local>,
    incoming_sizes:     ArenaMap<'s, usize, crate::ir::Value>,
    /// The block of each label, created at its first `goto` or definition.
    labels:             ArenaMap<'s, LabelId, Block>,
    targets:            ArenaMap<'s, StatementId, JumpTargets>,
    /// The case blocks of the open switches, sealed when each ends.
    switch_blocks:      ArenaVec<'s, Block>,
    /// Where each open switch's blocks start in `switch_blocks`.
    switch_starts:      ArenaVec<'s, usize>,
    /// The unqualified result type that `return` converts to.
    result:             TypeId,
    bounds:             ArenaMap<'s, usize, ssa::Var>,
    vla_tokens:         ArenaMap<'s, RegionId, ssa::Var>,
    /// The block computed gotos branch through, once one exists.
    indirect:           Option<Block>,
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
