//! The IR interpreter: executes a module directly, as the reference
//! semantics of the bcc IR and the oracle for differential tests.
//!
//! [`run`] loads the module into a [`Machine`], calls the entry function and
//! then executes one instruction per step until the entry function returns,
//! the program calls `exit` or `abort`, or it traps. Calls never recurse on
//! the native stack: the machine keeps an explicit stack of frames, each
//! naming its function, the next instruction to run, the base of its values
//! in one shared value stack, its stack slots, its variadic arguments, and
//! the value in its caller that receives its result. Returning pops the
//! frame, frees its slots and stores the result into the caller.
//!
//! Values are integers of each width (as masked `u128` bits), `f32`, `f64`
//! and x87 `f80` (computed bit for bit by `crate::x87`), pointers with
//! provenance, and poison, following the IR's LLVM-like
//! rules exactly: violated `nsw`, `nuw`, `exact` or `inbounds` facts give
//! poison, poison propagates, and branching on poison, dividing by it or
//! using it as an address traps as undefined behaviour. Memory is a set of
//! byte-addressed objects (stack slots, globals, heap allocations,
//! functions, each function's labels, `va_list` cursors); see `memory.rs`.
//! Calls to functions the module declares but does not define go to a [`Host`];
//! [`DefaultHost`] provides a small C library and captures the program's
//! output.
//!
//! For `function @main() -> i32` whose entry block computes `iadd.i32 v0,
//! v1` and returns it, `run` allocates the globals and function objects,
//! pushes a frame for `@main`, executes the `iconst`s, the `iadd` and the
//! `return`, pops the frame, and reports [`Termination::Returned`] with the
//! sum.
//!
//! Read [`run`] and [`Machine`] first, then `execute.rs` for what each
//! instruction does and `machine.rs` for calls, returns and branches.
//!
//! - Execution: `machine.rs` loads a module and moves between frames and
//!   blocks; `execute.rs` dispatches one instruction; `varargs.rs` implements
//!   `va_start`, `va_arg`, `va_copy` and `va_end`; `nonlocal.rs` `setjmp` and
//!   `longjmp`; `callbacks.rs` `qsort` and `bsearch`, which call back into the
//!   program. `main.rs` supplies writable arguments for hosted C entry;
//!   `startup.rs` orders and runs constructors and destructors around it,
//!   and `exit.rs` the exit functions and the handlers they run.
//! - Semantics of values: `value.rs` defines runtime values and pointers;
//!   `arithmetic.rs` the arithmetic, comparisons and conversions; `memory.rs`
//!   objects, loads, stores, copies and provenance.
//! - The outside world: `host.rs` defines the host interface and the default
//!   host; `outcome.rs` how a run ends and its limits; `trap.rs` why a run
//!   stops early.
//! - `text.rs` runs textual IR for tests; `tests.rs` and `tests/` exercise the
//!   interpreter.
//!
//! Limits: `f128` arithmetic traps as unsupported; `f128` representations
//! support constants, loads, stores, freeze and bitcasts.
//! `inttoptr` maps an address to the live object containing it, a deterministic
//! stand-in for a full provenance model. Declared globals without an
//! initializer have unresolved addresses; accessing their data traps.
//! Zero-initialized definitions, including tentative C definitions, have
//! storage.

// Execution
pub(crate) mod callbacks;
mod execute;
pub(crate) mod exit;
mod machine;
mod main;
mod nonlocal;
mod startup;
mod varargs;

// Semantics of values
mod arithmetic;
mod memory;
mod value;

// The outside world
mod host;
mod outcome;
mod trap;

// Tests
#[cfg(test)]
#[expect(
    clippy::disallowed_types,
    clippy::disallowed_macros,
    reason = "Tests build IR text with std strings; the arena rule covers the compiler only."
)]
mod tests;
#[cfg(test)]
#[expect(
    clippy::disallowed_types,
    clippy::disallowed_methods,
    reason = "Test helpers return owned output; the arena rule covers the compiler, not its tests."
)]
mod text;

pub(crate) use host::{
    Channel,
    DefaultHost,
    Host,
    HostCall,
    HostReturn,
};
pub(crate) use memory::{
    Memory,
    ObjectKind,
};
pub(crate) use outcome::{
    Limits,
    Outcome,
    Termination,
};
#[cfg(test)]
pub(crate) use text::{
    TextRun,
    run_text,
    run_text_with,
};
pub(crate) use trap::{
    EntryError,
    Fault,
    Location,
    Trap,
    UbKind,
};
pub(crate) use value::{
    Pointer,
    RuntimeValue,
};

use crate::{
    ir::{
        FuncId,
        Module,
        Type,
    },
    util::bump::{
        ArenaVec,
        Bump,
    },
};

/// Runs `entry` of `module` with `args` under the default [`Limits`].
///
/// The module must pass the verifier, in either profile.
pub(crate) fn run(
    module: &Module<'_>,
    entry: &str,
    args: &[RuntimeValue],
    host: &mut dyn Host,
) -> Result<Outcome, Trap> {
    run_with_limits(module, entry, args, host, Limits::default())
}

/// Runs hosted C main with no command-line arguments: argc is 1, argv[0]
/// names an unavailable program name (the empty string), and argv[1] is null.
/// Functions accepting no parameters keep the ordinary zero-argument entry.
/// The module's constructors run first and its destructors after main
/// returns or the program calls `exit`, in the order `startup` gives.
/// C99: §5.1.2.2.1 paragraphs 1-2, p. 12; PDF p. 24; §5.1.2.2.3, p. 13;
/// PDF p. 25.
#[cfg(test)]
pub(crate) fn run_main(module: &Module<'_>, host: &mut dyn Host) -> Result<Outcome, Trap> {
    run_main_with_limits(module, host, Limits::default())
}

/// Loads globals and relocations without executing constructors or user
/// instructions. The driver uses this before committing optional retention.
/// C99: §5.1.2p1, p. 11; PDF p. 23 (startup precedes execution).
pub(crate) fn check_module(
    module: &Module<'_>,
    host: &dyn Host,
    limits: Limits,
) -> Result<(), Trap> {
    let arena = Bump::new();
    Machine::load(module, &arena, limits, host).map(|_| ())
}

/// [`run_main`] stopping at `limits`.
pub(crate) fn run_main_with_limits(
    module: &Module<'_>,
    host: &mut dyn Host,
    limits: Limits,
) -> Result<Outcome, Trap> {
    let arena = Bump::new();
    let mut machine = Machine::load(module, &arena, limits, &*host)?;
    let (constructors, destructors) = startup::order(module, &arena);
    for &func in &constructors {
        machine.enter_function(func)?;
        let outcome = machine.run(host)?;
        match outcome.termination {
            | Termination::Returned(_) => {},
            | Termination::Exited(_) => return exit(&mut machine, &destructors, outcome, host),
            | Termination::Aborted => return Ok(outcome),
        }
    }
    machine.enter_main()?;
    let outcome = machine.run(host)?;
    if outcome.termination == Termination::Aborted {
        return Ok(outcome);
    }
    exit(&mut machine, &destructors, outcome, host)
}

/// Ends a program that returned from `main` or called an exit function:
/// the exit handlers and destructors run as its ending asks, and then the
/// host ends it normally, writing out its streams, or at once.
fn exit(
    machine: &mut Machine<'_, '_, '_>,
    destructors: &[FuncId],
    outcome: Outcome,
    host: &mut dyn Host,
) -> Result<Outcome, Trap> {
    let outcome = machine.finish(destructors, outcome, host)?;
    // A handler that aborts ends the program abnormally after all.
    match (outcome.termination, machine.ending()) {
        | (Termination::Aborted, _) => {},
        | (_, exit::Ending::Normal) => host.at_exit(),
        | (_, exit::Ending::Quick | exit::Ending::Immediate) => host.at_immediate_exit(),
    }
    Ok(outcome)
}

/// Runs `entry` of `module` with `args`, stopping at `limits`.
pub(crate) fn run_with_limits(
    module: &Module<'_>,
    entry: &str,
    args: &[RuntimeValue],
    host: &mut dyn Host,
    limits: Limits,
) -> Result<Outcome, Trap> {
    let arena = Bump::new();
    let mut machine = Machine::load(module, &arena, limits, &*host)?;
    machine.enter(entry, args)?;
    machine.run(host)
}

impl Machine<'_, '_, '_> {
    /// Executes an entered program until it terminates or traps.
    fn run(&mut self, host: &mut dyn Host) -> Result<Outcome, Trap> {
        loop {
            if let Some(termination) = self.step(host)? {
                return Ok(Outcome {
                    termination,
                    steps: self.steps,
                    #[cfg(test)]
                    memory_cells: self.memory.backing_cells(),
                });
            }
        }
    }

    /// Executes one instruction, saying whether the program ended.
    fn step(&mut self, host: &mut dyn Host) -> Result<Option<Termination>, Trap> {
        if self.limits.steps != 0 && self.steps == self.limits.steps {
            return Err(Trap::StepLimit);
        }
        // Unlimited execution must not overflow its observation counter.
        self.steps = self.steps.saturating_add(1);
        let (location, inst) = self.fetch();
        self.execute(inst, host)
            .map_err(|fault| self.trap(fault, location))
    }
}

/// The interpreter's state while it runs one program.
struct Machine<'m, 'ir, 'a> {
    module:         &'m Module<'ir>,
    limits:         Limits,
    memory:         Memory<'a>,
    /// The call stack; the last frame is executing.
    frames:         ArenaVec<'a, machine::Frame>,
    /// Every frame's values, indexed from the frame's base by `Value`.
    values:         ArenaVec<'a, RuntimeValue>,
    /// Dynamic objects released by stack restoration or return.
    dynamic_slots:  ArenaVec<'a, Pointer>,
    /// Every frame's stack-slot objects, from the frame's base.
    slots:          ArenaVec<'a, Pointer>,
    /// Every frame's variadic arguments with their types.
    varargs:        ArenaVec<'a, (Type, RuntimeValue, Option<crate::ir::ParamAbi<'ir>>)>,
    /// The arguments of the call or edge being made.
    arguments:      ArenaVec<'a, RuntimeValue>,
    /// The ABI descriptions of a call's variadic objects.
    argument_abis:  ArenaVec<'a, Option<crate::ir::ParamAbi<'ir>>>,
    /// The types of a call's arguments, for its variadic ones.
    argument_types: ArenaVec<'a, Type>,
    /// The values a host call's pre-ABI `va_list` operand has left.
    host_varargs:   ArenaVec<'a, RuntimeValue>,
    /// The address of each function, by `FuncId`.
    functions:      ArenaVec<'a, Pointer>,
    /// The address of label 0 of each function that has labels, by
    /// `FuncId`; label `n` is `n` bytes further.
    labels:         ArenaVec<'a, Option<Pointer>>,
    /// The address of each global, by `GlobalId`.
    globals:        ArenaVec<'a, Pointer>,
    /// Instructions executed so far.
    steps:          u64,
    /// The number the next frame's activation gets.
    activations:    u64,
    /// The last value `read_timestamp` produced.
    timestamp:      u64,
    /// While a C library function runs a comparison function: the frame
    /// depth its frame returns to, then the value it returned.
    callback:       Option<callbacks::Callback>,
    /// The exit handlers and how the program is ending.
    exit:           exit::Exit<'a>,
}
