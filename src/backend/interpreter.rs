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
//! Values are integers of each width (as masked `u128` bits), `f32` and
//! `f64`, pointers with provenance, and poison, following the IR's LLVM-like
//! rules exactly: violated `nsw`, `nuw`, `exact` or `inbounds` facts give
//! poison, poison propagates, and branching on poison, dividing by it or
//! using it as an address traps as undefined behaviour. Memory is a set of
//! byte-addressed objects (stack slots, globals, heap allocations,
//! functions, `va_list` cursors); see `memory.rs`. Calls to functions the
//! module declares but does not define go to a [`Host`]; [`DefaultHost`]
//! provides a small C library and captures the program's output.
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
//!   `va_start`, `va_arg`, `va_copy` and `va_end`.
//! - Semantics of values: `value.rs` defines runtime values and pointers;
//!   `arithmetic.rs` the arithmetic, comparisons and conversions; `memory.rs`
//!   objects, loads, stores, copies and provenance.
//! - The outside world: `host.rs` defines the host interface and the default
//!   host; `outcome.rs` how a run ends and its limits; `trap.rs` why a run
//!   stops early.
//! - `text.rs` runs textual IR for tests; `tests.rs` and `tests/` exercise the
//!   interpreter.
//!
//! Limits: `f80` and `f128` values trap as unsupported. `inttoptr` maps an
//! address to the live object containing it, a deterministic stand-in for a
//! full provenance model. Declared globals without an initializer are
//! zero-filled objects of their declared size.

// Execution
mod execute;
mod machine;
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

/// Runs `entry` of `module` with `args`, stopping at `limits`.
pub(crate) fn run_with_limits(
    module: &Module<'_>,
    entry: &str,
    args: &[RuntimeValue],
    host: &mut dyn Host,
    limits: Limits,
) -> Result<Outcome, Trap> {
    let arena = Bump::new();
    let mut machine = Machine::load(module, &arena, limits)?;
    machine.enter(entry, args)?;
    loop {
        if machine.steps == limits.steps {
            return Err(Trap::StepLimit);
        }
        machine.steps += 1;
        let (location, inst) = machine.fetch();
        match machine.execute(inst, host) {
            | Ok(None) => {},
            | Ok(Some(termination)) =>
                return Ok(Outcome {
                    termination,
                    steps: machine.steps,
                }),
            | Err(fault) => return Err(machine.trap(fault, location)),
        }
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
    /// Every frame's stack-slot objects, from the frame's base.
    slots:          ArenaVec<'a, Pointer>,
    /// Every frame's variadic arguments with their types.
    varargs:        ArenaVec<'a, (Type, RuntimeValue)>,
    /// The arguments of the call or edge being made.
    arguments:      ArenaVec<'a, RuntimeValue>,
    /// The types of a call's arguments, for its variadic ones.
    argument_types: ArenaVec<'a, Type>,
    /// The address of each function, by `FuncId`.
    functions:      ArenaVec<'a, Pointer>,
    /// The address of each global, by `GlobalId`.
    globals:        ArenaVec<'a, Pointer>,
    /// Instructions executed so far.
    steps:          u64,
    /// The number the next frame's activation gets.
    activations:    u64,
}
